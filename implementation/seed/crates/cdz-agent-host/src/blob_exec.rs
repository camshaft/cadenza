//! The `blob/*` executor — content-addressed store PUT/GET, the generic CAS-write MECHANISM (cadenza-docs I3:
//! a content reducer — e.g. docs-in-harness — writes a blob + gets back its content hash to reference it).
//! Serves the kernel's `blob/put` + `blob/get` effect families (the `fs/*` twin, for content-addressed
//! storage instead of paths).
//!
//! **THIN MECHANISM ONLY (operator standing-order: minimize host logic — the host is INEVOLVABLE).** This
//! executor does only the store ops — put bytes → hash, get bytes by hash. It carries NO policy: WHICH blobs
//! a session may put/get is EVOLVABLE POLICY in the Cedar WASM policy on the log (the kernel authorizes each
//! `blob/*` effect on its resolved target before dispatch, SEC-F1), NOT baked into this inevolvable host code.
//!
//! **Wire shape (reconciled with v-agent-harness, the kernel-family owner).** The hash is carried as a HEX
//! string, because a reducer threads `blob/put`'s result-hash into `blob/get`'s TARGET — and a target is now
//! opaque bytes ([`EffectRequest::target`] is `Arc<[u8]>`), read as UTF-8 via [`EffectRequest::target_str`], so
//! a text handle is used (hex is self-evidently the hash; reuses [`Hash::to_hex`]/[`Hash::from_hex`]). A
//! non-UTF-8 target is a clean fail-closed `Err`. NO event_ast codec — the hex string +
//! `EffectOutcome`'s `Option<Payload>` cover it:
//! - `blob/put`: payload = the content bytes (`Payload::Inline`); `Ok` result = `Inline(hash.to_hex())`.
//! - `blob/get`: target = the hex hash → `Hash::from_hex` it (a bad/non-hex target = a clean `Err`); a HIT =
//!   `Ok(Some(Inline(bytes)))`, a MISS = `Ok(None)` (a CAS get-miss is a NORMAL answer the reducer folds, not
//!   an error — mirrors `BlobStore::get`'s own `Ok(None)`); a backend I/O error OR a corrupt-blob integrity
//!   failure (`BlobStore::get`'s `Err(InvalidData)` on a hash mismatch — tamper-detection) stays `Err`.
//!
//! **Generic over the [`BlobStore`]** so a test drives a `MemBlobStore` and the deployed daemon selects the
//! S3 / disk backend by config (the same store the reducer-component blob store uses). NOT feature-gated: the
//! store trait + `MemBlobStore` are always-on (hermetic), so this executor compiles in the default build; the
//! S3 backend it can wrap is the only `live-aws-storage` piece.

use cdz_kernel::blob::BlobStore;
use cdz_kernel::effect::{effect_ct, EffectId, EffectRequest, Payload};
use cdz_kernel::event::EffectOutcome;
use cdz_kernel::executor::Executor;
use cdz_kernel::hash::Hash;

/// A `blob/*` executor over a [`BlobStore`] `B`. Owns the store (mutable — `put` takes `&mut`), makes no
/// policy decision (the kernel Cedar-authorized the effect's target first). Serves `blob/put` + `blob/get`.
pub struct BlobExecutor<B: BlobStore> {
    store: B,
}

impl<B: BlobStore> BlobExecutor<B> {
    /// Construct over a store. No configuration — WHICH blobs are reachable is the Cedar policy's call,
    /// decided before this executor is reached, not here.
    pub fn new(store: B) -> Self {
        BlobExecutor { store }
    }
}

#[async_trait::async_trait(?Send)]
impl<B: BlobStore> Executor for BlobExecutor<B> {
    async fn perform(
        &mut self,
        _id: EffectId,
        req: &EffectRequest,
        _idempotency_key: Hash,
    ) -> EffectOutcome {
        let family = req.content_type.family.as_ref();
        if family == effect_ct::BLOB_PUT {
            // Store the payload bytes → return the content hash as a HEX handle the reducer threads to get.
            let bytes = match &req.payload {
                Some(Payload::Inline(b)) => b.clone(),
                // A blob/put with no inline payload has nothing to store; a blob-ref payload would need this
                // executor to already resolve a hash (circular) — both are malformed → PERMANENT.
                Some(Payload::Blob(_)) => {
                    return EffectOutcome::err(
                        "blob/put: a blob-ref payload is unsupported — inline the bytes to store",
                    );
                }
                None => return EffectOutcome::err("blob/put: no payload bytes to store"),
            };
            // Compute the content hash ONCE here (put no longer computes it — the hash is threaded to each
            // storage tier so a multi-tier write doesn't re-blake3 the same bytes). Move the ref-counted
            // `Bytes` into `put`; still report the hash to the reducer as the hex handle it threads to get.
            let hash = Hash::of(&bytes);
            match self.store.put(hash, bytes).await {
                Ok(()) => {
                    EffectOutcome::Ok(Some(Payload::Inline(hash.to_hex().into_bytes().into())))
                }
                Err(e) => EffectOutcome::err(format!("blob/put failed: {e}")),
            }
        } else if family == effect_ct::BLOB_GET {
            // Target = the hex hash. A non-UTF-8 target (target is now opaque Arc<[u8]>) OR a
            // bad/non-hex/wrong-length one is a structural error → PERMANENT (fail-closed).
            let Ok(target) = req.target_str() else {
                return EffectOutcome::err(
                    "blob/get: target is not valid UTF-8 (expected a hex content hash)",
                );
            };
            let Some(hash) = Hash::from_hex(target) else {
                return EffectOutcome::err(format!(
                    "blob/get: target {target:?} is not a valid hex content hash"
                ));
            };
            match self.store.get(&hash).await {
                // HIT → the stored bytes verbatim (already ref-counted `Bytes`, moved straight into the payload).
                Ok(Some(bytes)) => EffectOutcome::Ok(Some(Payload::Inline(bytes))),
                // MISS → Ok(None): "ran fine, no such blob" (a normal answer the reducer folds), NOT an Err.
                Ok(None) => EffectOutcome::Ok(None),
                // Backend I/O error OR a corrupt-blob integrity failure (Err(InvalidData) on a hash mismatch —
                // tamper-detection). PERMANENT: a re-fetch of the same corrupt/absent-backend blob re-fails.
                Err(e) => EffectOutcome::err(format!("blob/get failed: {e}")),
            }
        } else {
            // Not a blob family — structural (a CompositeExecutor routes by family, so this is a mis-route).
            EffectOutcome::err(format!(
                "BlobExecutor only handles the {} family, got {family}",
                effect_ct::BLOB_PREFIX
            ))
        }
    }

    /// Serves the `blob/*` families (both put + get share this executor).
    fn handles_family(&self, family: &str) -> bool {
        family == effect_ct::BLOB_PUT || family == effect_ct::BLOB_GET
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_kernel::blob::MemBlobStore;
    use cdz_kernel::effect::Timeliness;

    fn put_req(bytes: &[u8]) -> EffectRequest {
        EffectRequest::new_with_family(
            effect_ct::BLOB_PUT,
            String::new(),
            Some(Payload::Inline(bytes.to_vec().into())),
            Timeliness::Interactive,
        )
    }

    fn get_req(hex: &str) -> EffectRequest {
        EffectRequest::new_with_family(effect_ct::BLOB_GET, hex, None, Timeliness::Interactive)
    }

    #[tokio::test]
    async fn put_returns_a_hex_hash_and_get_round_trips_the_bytes() {
        let mut exec = BlobExecutor::new(MemBlobStore::new());
        // put → hex hash handle
        let hex = match exec
            .perform(EffectId(0), &put_req(b"doc-ast-bytes"), Hash::of(b"k"))
            .await
        {
            EffectOutcome::Ok(Some(Payload::Inline(b))) => String::from_utf8(b.to_vec()).unwrap(),
            other => panic!("blob/put should return a hex hash, got {other:?}"),
        };
        assert!(
            Hash::from_hex(&hex).is_some(),
            "the put result is a valid hex content hash: {hex:?}"
        );
        // get(that hash) → the bytes verbatim
        match exec
            .perform(EffectId(0), &get_req(&hex), Hash::of(b"k"))
            .await
        {
            EffectOutcome::Ok(Some(Payload::Inline(b))) => {
                assert_eq!(
                    b.as_ref(),
                    b"doc-ast-bytes",
                    "get round-trips the stored bytes"
                );
            }
            other => panic!("blob/get should return the stored bytes, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn get_of_an_absent_blob_is_ok_none_not_err() {
        let mut exec = BlobExecutor::new(MemBlobStore::new());
        // A valid-hex hash that was never stored → Ok(None), NOT an Err (a CAS miss is a normal fold input).
        let absent = Hash::of(b"never-stored").to_hex();
        match exec
            .perform(EffectId(0), &get_req(&absent), Hash::of(b"k"))
            .await
        {
            EffectOutcome::Ok(None) => {}
            other => panic!("an absent blob must be Ok(None), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn get_with_a_non_hex_target_is_a_permanent_err() {
        use cdz_kernel::event::Retryability;
        let mut exec = BlobExecutor::new(MemBlobStore::new());
        match exec
            .perform(EffectId(0), &get_req("not-a-hex-hash"), Hash::of(b"k"))
            .await
        {
            EffectOutcome::Err {
                retryability: Retryability::Permanent,
                ..
            } => {}
            other => panic!("a non-hex blob/get target must be a permanent Err, got {other:?}"),
        }
    }

    // ---- doc-publish→query round-trip through the REAL host stack (converted from the deleted
    // doc_publish_i3_e2e integration test, operator no-integration-tests mandate — hermetic: a live
    // BlobExecutor over a shared content-addressed store + the kernel's store/* name-directory + a Cedar-style
    // authorizer, no S3/network). Proves the BlobExecutor composes in a real multi-effect fold (blob/put ->
    // store/set index -> store/resolve -> blob/get recovers), not just the isolated put/get above. ----
    use crate::host::HostedSession;
    use cdz_kernel::authz::Authorizer;
    use cdz_kernel::effect::{Capability, ResourcePredicate};
    use cdz_kernel::event::{ContentType, Event, EventBody};
    use cdz_kernel::event_ast::{decode_name_set, encode_name_set};
    use cdz_kernel::executor::CompositeExecutor;
    use cdz_kernel::kv::Kv;
    use cdz_kernel::name_store::NameStore;
    use cdz_kernel::reducer::{FoldOutput, Reducer};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    /// A SHARED in-memory content-addressed store — the two blob/put + blob/get `BlobExecutor`s (a
    /// `CompositeExecutor` routes one executor per family) must see EACH OTHER's writes, exactly as the
    /// deployed daemon's two per-verb `S3BlobStore`s share one bucket. `MemBlobStore` is a per-instance
    /// HashMap, so this `Arc<Mutex<HashMap>>`-backed store models S3's shared-bucket semantics hermetically.
    #[derive(Clone, Default)]
    struct SharedMemBlobStore {
        blobs: Arc<Mutex<HashMap<Hash, bytes::Bytes>>>,
    }

    #[async_trait::async_trait(?Send)]
    impl BlobStore for SharedMemBlobStore {
        async fn put(&mut self, hash: Hash, bytes: bytes::Bytes) -> std::io::Result<()> {
            // The content hash is SUPPLIED (computed once by the caller) — store under it, don't re-hash.
            self.blobs.lock().unwrap().insert(hash, bytes);
            Ok(())
        }
        async fn get(&self, hash: &Hash) -> std::io::Result<Option<bytes::Bytes>> {
            Ok(self.blobs.lock().unwrap().get(hash).cloned())
        }
    }

    /// The doc-index reducer (the shape a real cadenza-docs reducer folds). PUBLISH: `doc/publish` inbound ->
    /// `blob/put` the doc-AST -> (hex hash) -> `store/set` `memory/doc/<pkg>`. QUERY: `doc/query` inbound ->
    /// `store/resolve` -> (name-set) -> `blob/get` the hash -> doc-AST bytes recovered into KV.
    struct DocPublishReducer {
        name: &'static str, // e.g. "memory/doc/cadenza-syntax"
    }

    #[async_trait::async_trait(?Send)]
    impl Reducer for DocPublishReducer {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
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
                            "doc",
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
                        // Phase 1: blob/put returned the hex hash -> register it at the doc name in the index.
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
                        // Phase 2: store/resolve returned the (name, hash) -> fetch the doc bytes from the CAS.
                        FoldOutput::with(vec![EffectRequest::new_with_family(
                            effect_ct::BLOB_GET,
                            h.to_hex(),
                            None,
                            Timeliness::Interactive,
                        )])
                    } else {
                        // Phase 3: blob/get returned the doc-AST bytes -> recovered.
                        kv.put(b"recovered".to_vec(), bytes.to_vec());
                        FoldOutput::none()
                    }
                }
                _ => FoldOutput::none(),
            }
        }
    }

    /// The doc reducer's authorizer: the CAS verbs (blob/put + blob/get) + the doc-index name verbs
    /// (store/set + store/resolve on `memory/`, the writable promotion scope where docs register).
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

    fn doc_inbound(family: &'static str, payload: Option<Vec<u8>>) -> EventBody {
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

        // A live BlobExecutor per blob verb over a SHARED content-addressed store (the daemon wires the SAME
        // BlobExecutor over two per-verb S3BlobStores pointing at ONE bucket). store/* is applied by the kernel
        // against the attached NameStore, so only blob/* need executors.
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

        // PUBLISH: one deliver drives blob/put -> (hex) -> store/set memory/doc/<pkg> to quiescence.
        session
            .deliver(doc_inbound("doc/publish", Some(doc_ast.clone())), None)
            .await
            .expect("publish delivers");
        assert_eq!(
            session.session().open_effects(),
            0,
            "the publish chain (blob/put -> store/set) settled"
        );
        let entries = session
            .session()
            .name_store()
            .expect("name store attached")
            .to_set_entries();
        assert!(
            entries.iter().any(|(n, _h)| n == name),
            "the doc registered at {name} in the index: {entries:?}"
        );

        // QUERY: one deliver drives store/resolve -> blob/get -> recovered.
        session
            .deliver(doc_inbound("doc/query", None), None)
            .await
            .expect("query delivers");
        assert_eq!(
            session.session().open_effects(),
            0,
            "the query chain (store/resolve -> blob/get) settled"
        );
        let recovered = session
            .session()
            .kv()
            .get(b"recovered")
            .expect("the query recovered the doc bytes into KV");
        assert_eq!(
            recovered, doc_ast,
            "the queried doc-AST bytes round-trip the published ones through the real blob executor + index"
        );
    }
}
