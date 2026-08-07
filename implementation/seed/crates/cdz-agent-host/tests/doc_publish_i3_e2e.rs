//! End-to-end (hermetic): the cadenza-docs I3 doc-publish→query round-trip, driven through the REAL host
//! stack — a `DocPublishReducer` folds `doc/publish` → `blob/put`(doc-AST) → hex hash → `store/set`
//! `memory/doc/<pkg>`, then a later `doc/query` → `store/resolve` → `blob/get` recovers the SAME doc-AST
//! bytes. This is the HOST-side counterpart to v-agent-harness's kernel fold-proof
//! (`doc_publish_index_round_trips_…`): that proves the fold composes at the kernel level with a STUB blob
//! executor; THIS drives the identical fold through the real machinery — a live [`BlobExecutor`] over a
//! content-addressed store + the kernel's `store/*` name-directory + a Cedar-style authorizer granting
//! `memory/` writes — exactly as the deployed daemon runs it (the daemon wires the same BlobExecutor over an
//! S3 store; here MemBlobStore keeps it hermetic).
//!
//! Naming (corpus-bugfix ruling): docs register at `memory/doc/<pkg>` — `memory/` is the writable durable
//! cross-session scope (a promotion-authority prefix), so the reducer needs a `memory/`-write grant. `doc/`
//! alone is Unscoped/unwritable. So the session authorizer grants `store/set`+`store/resolve` on `memory/`
//! (the doc index) + `blob/put`+`blob/get` (the CAS), the exact capability shape a real doc reducer's policy
//! carries.
//!
//! Hermetic (no feature gate): BlobExecutor is generic over the store + MemBlobStore is always-on, so this
//! runs in the default gate (no S3/network). The doc-AST is opaque bytes here (the point is the CAS+index
//! round-trip, not doc parsing).

use cdz_agent_host::{BlobExecutor, HostedSession};
use cdz_kernel::authz::Authorizer;
use cdz_kernel::blob::BlobStore;
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
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A SHARED in-memory content-addressed store — the two `blob/put`/`blob/get` `BlobExecutor`s (a
/// `CompositeExecutor` routes one executor per family) must see EACH OTHER's writes, exactly as the deployed
/// daemon's two per-verb `S3BlobStore`s share one bucket. `MemBlobStore` is a plain per-instance HashMap
/// (no shared storage), so this Arc<Mutex<HashMap>>-backed store models S3's shared-bucket semantics
/// hermetically: a clone shares the same map (put via one handle is visible to get via the other). Self-
/// verifying like the real stores (get re-hashes + refuses a mismatch).
#[derive(Clone, Default)]
struct SharedMemBlobStore {
    blobs: Arc<Mutex<HashMap<Hash, Vec<u8>>>>,
}

#[async_trait::async_trait(?Send)]
impl BlobStore for SharedMemBlobStore {
    async fn put(&mut self, bytes: &[u8]) -> std::io::Result<Hash> {
        let hash = Hash::of(bytes);
        self.blobs.lock().unwrap().insert(hash, bytes.to_vec());
        Ok(hash)
    }
    async fn get(&self, hash: &Hash) -> std::io::Result<Option<Vec<u8>>> {
        Ok(self.blobs.lock().unwrap().get(hash).cloned())
    }
}

/// The doc-index reducer — mirrors v-agent-harness's kernel fold-proof `DocPublishReducer` (the shape a real
/// cadenza-docs reducer folds). PUBLISH: `doc/publish` inbound → `blob/put` the doc-AST → (result: hex hash)
/// → `store/set` `memory/doc/<pkg>` = a name-set pointer at that hash. QUERY: `doc/query` inbound →
/// `store/resolve` the name → (result: name-set) → `blob/get` the hash → (result: doc-AST bytes) recovered.
struct DocPublishReducer {
    name: &'static str, // e.g. "memory/doc/cadenza-syntax"
}

#[async_trait::async_trait(?Send)]
impl Reducer for DocPublishReducer {
    async fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
        match &event.body {
            EventBody::Inbound {
                content_type,
                payload,
            } => match content_type.family.as_ref() {
                "doc/publish" => {
                    let doc = match payload {
                        Payload::Inline(b) => b.clone(),
                        Payload::Blob(_) => return FoldOutput::none(),
                    };
                    FoldOutput::with(vec![EffectRequest::new_with_family(
                        effect_ct::BLOB_PUT,
                        "doc", // the label the blob write authorizes on (the CAS keys by content)
                        Some(Payload::Inline(doc)),
                        Timeliness::Interactive,
                    )])
                }
                "doc/query" => FoldOutput::with(vec![EffectRequest::new_with_family(
                    effect_ct::STORE_RESOLVE,
                    self.name,
                    None,
                    Timeliness::Interactive,
                )]),
                _ => FoldOutput::none(),
            },
            EventBody::EffectResult {
                result: EffectOutcome::Ok(Some(Payload::Inline(bytes))),
                ..
            } => {
                if kv.get(b"published").is_none() {
                    // Phase 1: blob/put returned the hex hash → register it at the doc name in the index.
                    kv.put(b"published".to_vec(), bytes.to_vec());
                    let hex = String::from_utf8_lossy(bytes).into_owned();
                    let hash = match Hash::from_hex(&hex) {
                        Some(h) => h,
                        None => return FoldOutput::none(),
                    };
                    let payload = encode_name_set(self.name, &hash);
                    FoldOutput::with(vec![EffectRequest::new_with_family(
                        effect_ct::STORE_SET,
                        self.name,
                        Some(Payload::Inline(payload.into())),
                        Timeliness::Interactive,
                    )])
                } else if let Ok((_n, h)) = decode_name_set(bytes) {
                    // Phase 2: store/resolve returned the (name, hash) → fetch the doc bytes from the CAS.
                    FoldOutput::with(vec![EffectRequest::new_with_family(
                        effect_ct::BLOB_GET,
                        h.to_hex(),
                        None,
                        Timeliness::Interactive,
                    )])
                } else {
                    // Phase 3: blob/get returned the doc-AST bytes → recovered.
                    kv.put(b"recovered".to_vec(), bytes.to_vec());
                    FoldOutput::none()
                }
            }
            _ => FoldOutput::none(),
        }
    }
}

/// The doc reducer's authorizer: grant the CAS verbs (blob/put + blob/get) + the doc-index name verbs
/// (store/set + store/resolve on `memory/`, the writable promotion scope where docs register). This is the
/// capability shape a real cadenza-docs reducer's Cedar policy carries.
fn doc_authz() -> Authorizer {
    Authorizer::new(vec![]).with_family_grants(vec![
        Capability::for_family(effect_ct::BLOB_PUT, ResourcePredicate::Any),
        Capability::for_family(effect_ct::BLOB_GET, ResourcePredicate::Any),
        Capability::for_family(
            effect_ct::STORE_SET,
            ResourcePredicate::Prefix("memory/".into()),
        ),
        Capability::for_family(
            effect_ct::STORE_RESOLVE,
            ResourcePredicate::Prefix("memory/".into()),
        ),
    ])
}

fn inbound(family: &'static str, payload: Option<Vec<u8>>) -> EventBody {
    EventBody::Inbound {
        content_type: ContentType {
            family: family.into(),
            version: 1,
        },
        payload: payload.map_or(Payload::Inline(Vec::new().into()), |b| {
            Payload::Inline(b.into())
        }),
    }
}

#[tokio::test]
async fn a_doc_publish_then_query_round_trips_the_doc_ast_through_the_real_blob_executor_and_name_index(
) {
    let doc_ast = b"(doc-module (doc-item (name parse) (summary \"parse source\")))".to_vec();
    let name = "memory/doc/cadenza-syntax";

    // The real host executor set: a live BlobExecutor per blob verb over a SHARED content-addressed store
    // (hermetic; the daemon wires the SAME BlobExecutor over two per-verb S3BlobStores pointing at ONE
    // bucket). A CompositeExecutor routes one executor per family, so blob/put + blob/get get separate
    // BlobExecutors — but both wrap a CLONE of the same SharedMemBlobStore, so the put's write is visible to
    // the get (modeling the shared S3 bucket). store/* is applied by the kernel against the attached
    // NameStore, so no store executor is registered — only blob/*.
    let store = SharedMemBlobStore::default();
    let executor = CompositeExecutor::new()
        .with_effect(
            effect_ct::BLOB_PUT,
            Box::new(BlobExecutor::new(store.clone())),
        )
        .with_effect(
            effect_ct::BLOB_GET,
            Box::new(BlobExecutor::new(store.clone())),
        );

    let mut session = HostedSession::genesis(
        Hash::of(b"doc-publish-v1"),
        Box::new(DocPublishReducer { name }),
        Box::new(doc_authz()),
        executor,
    )
    .with_name_store(NameStore::new());

    // PUBLISH: one deliver drives the whole chain to quiescence — blob/put → (hex) → store/set memory/doc/<pkg>.
    session
        .deliver(inbound("doc/publish", Some(doc_ast.clone())), None)
        .await
        .expect("publish delivers");
    assert_eq!(
        session.session().open_effects(),
        0,
        "the publish chain (blob/put → store/set) settled"
    );
    // The doc name now resolves in the index.
    let entries = session
        .session()
        .name_store()
        .expect("name store attached")
        .to_set_entries();
    assert!(
        entries.iter().any(|(n, _h)| n == name),
        "the doc registered at {name} in the index: {entries:?}"
    );

    // QUERY: one deliver drives store/resolve → blob/get → recovered.
    session
        .deliver(inbound("doc/query", None), None)
        .await
        .expect("query delivers");
    assert_eq!(
        session.session().open_effects(),
        0,
        "the query chain (store/resolve → blob/get) settled"
    );
    let recovered = session
        .session()
        .kv()
        .get(b"recovered")
        .expect("the query recovered the doc bytes into KV");
    assert_eq!(
        recovered, doc_ast,
        "the queried doc-AST bytes round-trip the published ones through the real blob executor + name index"
    );
}
