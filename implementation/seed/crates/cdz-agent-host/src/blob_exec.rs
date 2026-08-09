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
                Some(Payload::Inline(b)) => b.as_ref(),
                // A blob/put with no inline payload has nothing to store; a blob-ref payload would need this
                // executor to already resolve a hash (circular) — both are malformed → PERMANENT.
                Some(Payload::Blob(_)) => {
                    return EffectOutcome::err(
                        "blob/put: a blob-ref payload is unsupported — inline the bytes to store",
                    );
                }
                None => return EffectOutcome::err("blob/put: no payload bytes to store"),
            };
            match self.store.put(bytes).await {
                Ok(hash) => {
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
                // HIT → the stored bytes verbatim.
                Ok(Some(bytes)) => EffectOutcome::Ok(Some(Payload::Inline(bytes.into()))),
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
}
