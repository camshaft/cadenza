//! A composable tiered [`BlobStore`] (`TieredBlobStore`) — a cache hierarchy over a durable floor.
//!
//! It holds an ordered list of layers, shallowest (fastest, e.g. in-memory) → deepest (durable, e.g. S3),
//! each itself just a [`BlobStore`]. That is the whole "composable" idea: every tier is the same trait, so
//! they nest arbitrarily and this combinator is one too.
//!
//! - **`get`** (read-through): try layers shallow→deep. On a hit at layer *i*, POPULATE the shallower
//!   layers `0..i` (best-effort — a cache-fill failure never fails the read) and return the bytes, so the
//!   next read is warm. An upper-layer *error* is not a miss, but it also mustn't hide a deeper hit, so a
//!   layer error is remembered and the search falls through to deeper layers; only if every layer
//!   misses-or-errors is an error surfaced (else `Ok(None)`).
//! - **`has`**: shallow→deep, short-circuit on the first present layer (write-through means any hit implies
//!   the bytes reached the durable floor).
//! - **`put`** (write-through, DEEPEST-FIRST): write the deepest (durable) layer FIRST — its failure fails
//!   the whole put, because the bytes were not durably persisted ("persist all the way to S3"). Then
//!   populate the shallower cache layers best-effort. So a returned `Ok` means the bytes are durable.
//!
//! Slice A: the combinator itself, exercised with in-memory fakes. The concrete tiers (a bounded in-memory
//! cache, a size-capped disk cache, an S3 floor) are follow-on slices — each is just a `BlobStore` plugged
//! into `TieredBlobStore::new`.

use async_trait::async_trait;
use bytes::Bytes;
use cdz_platform::{BlobStore, BlobStoreError, Hash};
use std::sync::Arc;

/// A [`BlobStore`] composed of ordered layers, shallowest → deepest. Reads fall through and warm the upper
/// layers; writes go through to the durable deepest layer first.
pub struct TieredBlobStore {
    /// Shallowest (index 0, fastest cache) → deepest (last, durable floor). Must be non-empty in practice;
    /// an empty stack behaves as an empty store (get→`Ok(None)`, has→`Ok(false)`, put→`Err` — nowhere to
    /// persist).
    layers: Vec<Arc<dyn BlobStore>>,
}

impl TieredBlobStore {
    /// Build a tiered store from `layers` ordered shallowest → deepest (e.g. `[mem_cache, disk_cache,
    /// s3]`). The last layer is the durable floor a `put` must reach.
    #[must_use]
    pub fn new(layers: Vec<Arc<dyn BlobStore>>) -> Self {
        Self { layers }
    }
}

#[async_trait]
impl BlobStore for TieredBlobStore {
    async fn put(&self, bytes: Bytes) -> Result<Hash, BlobStoreError> {
        let Some((deepest, uppers)) = self.layers.split_last() else {
            return Err(BlobStoreError::Io(
                "tiered blob store has no layers to persist to".to_string(),
            ));
        };
        // Deepest FIRST: the durable floor must accept the write, else the put failed (nothing persisted).
        let hash = deepest.put(bytes.clone()).await?;
        tracing::debug!(
            layers = self.layers.len(),
            "tiered put: durable, writing through caches"
        );
        // Then warm the shallower cache layers, best-effort — a cache-fill failure doesn't fail the put,
        // since the bytes are already durable in the deepest layer.
        for upper in uppers {
            let _ = upper.put(bytes.clone()).await;
        }
        Ok(hash)
    }

    async fn get(&self, hash: Hash) -> Result<Option<Bytes>, BlobStoreError> {
        let mut last_err: Option<BlobStoreError> = None;
        for (i, layer) in self.layers.iter().enumerate() {
            match layer.get(hash).await {
                Ok(Some(bytes)) => {
                    tracing::debug!(tier = i, "tiered get: hit");
                    // Read-through: warm every shallower layer, best-effort.
                    for upper in &self.layers[..i] {
                        let _ = upper.put(bytes.clone()).await;
                    }
                    return Ok(Some(bytes));
                }
                Ok(None) => {}
                // A layer error isn't a miss, but it mustn't hide a deeper hit — remember it and fall
                // through to the deeper (more durable) layers.
                Err(err) => {
                    tracing::warn!(tier = i, error = %err, "tiered get: layer error, falling through");
                    last_err = Some(err);
                }
            }
        }
        // Every layer missed or errored: surface an error if any layer couldn't determine its answer,
        // otherwise it's a genuine miss.
        match last_err {
            Some(err) => Err(err),
            None => Ok(None),
        }
    }

    async fn has(&self, hash: Hash) -> Result<bool, BlobStoreError> {
        let mut last_err: Option<BlobStoreError> = None;
        for layer in &self.layers {
            match layer.has(hash).await {
                Ok(true) => return Ok(true),
                Ok(false) => {}
                Err(err) => last_err = Some(err),
            }
        }
        match last_err {
            Some(err) => Err(err),
            None => Ok(false),
        }
    }
}

/// A degenerate helper only used to build a `Hash` for tests without a store.
#[cfg(test)]
fn blob_hash(bytes: &[u8]) -> Hash {
    Hash::of(cdz_platform::HashTag::Blob, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_platform::InMemoryBlobStore;

    /// A `BlobStore` that always errors — to exercise fall-through + write-through error handling.
    struct FailingStore;
    #[async_trait]
    impl BlobStore for FailingStore {
        async fn put(&self, _bytes: Bytes) -> Result<Hash, BlobStoreError> {
            Err(BlobStoreError::Io("failing store".to_string()))
        }
        async fn get(&self, _hash: Hash) -> Result<Option<Bytes>, BlobStoreError> {
            Err(BlobStoreError::Io("failing store".to_string()))
        }
        async fn has(&self, _hash: Hash) -> Result<bool, BlobStoreError> {
            Err(BlobStoreError::Io("failing store".to_string()))
        }
    }

    #[tokio::test]
    async fn get_reads_through_and_warms_upper_layers() {
        let upper = Arc::new(InMemoryBlobStore::new());
        let deep = Arc::new(InMemoryBlobStore::new());
        // Seed ONLY the deep layer.
        let bytes = Bytes::from_static(b"only in the deep layer");
        let hash = deep.put(bytes.clone()).await.unwrap();
        assert!(!upper.has(hash).await.unwrap(), "upper starts cold");

        let tiered = TieredBlobStore::new(vec![upper.clone(), deep.clone()]);
        // The read hits the deep layer…
        assert_eq!(tiered.get(hash).await.unwrap(), Some(bytes.clone()));
        // …and warms the upper cache (read-through).
        assert_eq!(upper.get(hash).await.unwrap(), Some(bytes));
    }

    #[tokio::test]
    async fn put_writes_through_to_every_layer() {
        let upper = Arc::new(InMemoryBlobStore::new());
        let deep = Arc::new(InMemoryBlobStore::new());
        let tiered = TieredBlobStore::new(vec![upper.clone(), deep.clone()]);

        let bytes = Bytes::from_static(b"write me through");
        let hash = tiered.put(bytes.clone()).await.unwrap();
        assert_eq!(hash, blob_hash(&bytes));
        // Write-through: both the durable floor AND the cache hold it.
        assert_eq!(deep.get(hash).await.unwrap(), Some(bytes.clone()));
        assert_eq!(upper.get(hash).await.unwrap(), Some(bytes));
    }

    #[tokio::test]
    async fn get_and_has_report_a_genuine_miss() {
        let tiered = TieredBlobStore::new(vec![
            Arc::new(InMemoryBlobStore::new()),
            Arc::new(InMemoryBlobStore::new()),
        ]);
        let absent = blob_hash(b"never stored");
        assert_eq!(tiered.get(absent).await.unwrap(), None);
        assert!(!tiered.has(absent).await.unwrap());
    }

    #[tokio::test]
    async fn has_short_circuits_on_the_first_present_layer() {
        let upper = Arc::new(InMemoryBlobStore::new());
        let deep = Arc::new(InMemoryBlobStore::new());
        let hash = upper
            .put(Bytes::from_static(b"cached up top"))
            .await
            .unwrap();
        let tiered = TieredBlobStore::new(vec![upper, deep]);
        assert!(tiered.has(hash).await.unwrap());
    }

    #[tokio::test]
    async fn a_put_fails_if_the_durable_floor_fails() {
        // Deepest layer errors → the whole put errors (nothing was durably persisted).
        let tiered = TieredBlobStore::new(vec![
            Arc::new(InMemoryBlobStore::new()),
            Arc::new(FailingStore),
        ]);
        assert!(matches!(
            tiered.put(Bytes::from_static(b"x")).await,
            Err(BlobStoreError::Io(_))
        ));
    }

    #[tokio::test]
    async fn get_falls_through_an_erroring_upper_layer_to_a_deeper_hit() {
        // Upper layer errors, but the deep layer holds the blob — the read still succeeds (the upper error
        // doesn't hide the deeper hit).
        let deep = Arc::new(InMemoryBlobStore::new());
        let bytes = Bytes::from_static(b"below the broken cache");
        let hash = deep.put(bytes.clone()).await.unwrap();
        let tiered = TieredBlobStore::new(vec![Arc::new(FailingStore), deep]);
        assert_eq!(tiered.get(hash).await.unwrap(), Some(bytes));
    }

    #[tokio::test]
    async fn get_surfaces_an_error_when_all_layers_error() {
        let tiered = TieredBlobStore::new(vec![Arc::new(FailingStore), Arc::new(FailingStore)]);
        assert!(matches!(
            tiered.get(blob_hash(b"anything")).await,
            Err(BlobStoreError::Io(_))
        ));
    }
}
