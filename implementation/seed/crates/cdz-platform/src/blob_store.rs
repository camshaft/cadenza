//! The content-addressed blob store (`design/cadenza-platform.md` §8).
//!
//! There is exactly one store: a mapping from a [`Hash`] to its bytes. Its whole interface is: put bytes
//! (getting back their hash), get bytes by hash, and ask whether a hash is present. Everything the system
//! keeps by hash lives here — log blobs, large state values, contract declarations, wasm components — a
//! component is not special, it is bytes addressed by its hash like any other value.
//!
//! **The store is unpermissioned: the hash is the capability.** You cannot forge bytes for a hash, so
//! possessing a hash both names and authorizes reading its bytes — there is nothing to gate on a read.
//! Confidentiality lives one layer up, at name resolution (which hashes a reducer ever comes to hold).
//!
//! The operations are **async** so a disk/network-backed store (a local cache, S3) can fetch without
//! blocking the runtime — but they stay deterministic: `get(hash)` is a pure function of the hash
//! (content-addressed, the same bytes every time) and `put(bytes)` a pure function of the bytes, so
//! awaiting a fetch changes only timing, never the result. The trait is [`async_trait`] so a backend is a
//! dyn-safe swappable trait object, and the methods are runtime-agnostic (they only await), so they run
//! under tokio in production and under the Bach simulator in deterministic tests alike.

use crate::{Bytes, Hash};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;

/// A content-addressed blob store: hash <-> bytes. The one store of §8; backends (in-memory, disk, S3)
/// implement this and are swapped by reference. `Send + Sync` so it can be shared across the runtime's
/// concurrent tasks behind an `Arc`.
#[async_trait]
pub trait BlobStore: Send + Sync {
    /// Store `bytes` and return their content hash. Idempotent by construction: the hash is derived from
    /// the bytes, so putting the same bytes twice yields the same hash and simply re-stores identical
    /// content. (No `Result`: a well-formed backend's put is a pure function of its input; a fallible
    /// backend absorbs transient I/O internally, e.g. by retry — this layer stays deterministic per §8/§9.)
    async fn put(&self, bytes: Bytes) -> Hash;

    /// Fetch the bytes stored under `hash`, or `None` if the store does not hold it. `None` is genuine
    /// absence, not a transient failure.
    async fn get(&self, hash: Hash) -> Option<Bytes>;

    /// Whether `hash` is present in the store.
    async fn has(&self, hash: Hash) -> bool;
}

/// An in-memory [`BlobStore`] — a plain hash-map behind a `Mutex`. For tests and single-process use; the
/// smallest honest backend. Runtime-agnostic: the lock is a `std::sync::Mutex` held only across the map
/// operation (never across an `.await`), so this drives identically under tokio and under Bach.
#[derive(Default)]
pub struct InMemoryBlobStore {
    blobs: Mutex<HashMap<Hash, Bytes>>,
}

impl InMemoryBlobStore {
    /// An empty in-memory store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of distinct blobs held (by content hash). Handy for tests/introspection.
    #[must_use]
    pub fn len(&self) -> usize {
        self.blobs.lock().expect("blob-store mutex poisoned").len()
    }

    /// Whether the store holds no blobs.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.blobs
            .lock()
            .expect("blob-store mutex poisoned")
            .is_empty()
    }
}

#[async_trait]
impl BlobStore for InMemoryBlobStore {
    async fn put(&self, bytes: Bytes) -> Hash {
        let hash = Hash::of(&bytes);
        // O(1) Bytes clone into the map; the guard is dropped before returning (no await held across it).
        self.blobs
            .lock()
            .expect("blob-store mutex poisoned")
            .insert(hash, bytes);
        hash
    }

    async fn get(&self, hash: Hash) -> Option<Bytes> {
        // `cloned()` on a Bytes is an O(1) refcount bump, not a copy.
        self.blobs
            .lock()
            .expect("blob-store mutex poisoned")
            .get(&hash)
            .cloned()
    }

    async fn has(&self, hash: Hash) -> bool {
        self.blobs
            .lock()
            .expect("blob-store mutex poisoned")
            .contains_key(&hash)
    }
}

#[cfg(test)]
mod tests {
    use super::{BlobStore, InMemoryBlobStore};
    use crate::{Bytes, Hash};

    #[tokio::test]
    async fn put_returns_the_content_hash_and_get_round_trips() {
        let store = InMemoryBlobStore::new();
        let bytes = Bytes::from_static(b"the hash is the capability");
        let h = store.put(bytes.clone()).await;
        // put returns the content hash of exactly those bytes.
        assert_eq!(h, Hash::of(&bytes));
        // get by that hash returns the same bytes.
        assert_eq!(store.get(h).await, Some(bytes));
        assert!(store.has(h).await);
    }

    #[tokio::test]
    async fn get_and_has_report_absence() {
        let store = InMemoryBlobStore::new();
        let absent = Hash::of(b"never stored");
        assert_eq!(store.get(absent).await, None);
        assert!(!store.has(absent).await);
    }

    #[tokio::test]
    async fn put_is_idempotent_by_content() {
        let store = InMemoryBlobStore::new();
        let h1 = store.put(Bytes::from_static(b"same")).await;
        let h2 = store.put(Bytes::from_static(b"same")).await;
        // same bytes -> same hash, and only one blob is held.
        assert_eq!(h1, h2);
        assert_eq!(store.len(), 1);
        // distinct bytes -> a distinct hash + a second blob.
        let h3 = store.put(Bytes::from_static(b"different")).await;
        assert_ne!(h1, h3);
        assert_eq!(store.len(), 2);
    }

    #[tokio::test]
    async fn stores_and_distinguishes_many_blobs() {
        let store = InMemoryBlobStore::new();
        let mut hashes = Vec::new();
        for i in 0..64u16 {
            hashes.push(
                store
                    .put(Bytes::from(format!("blob-{i}").into_bytes()))
                    .await,
            );
        }
        assert_eq!(store.len(), 64);
        // every stored blob is retrievable and the hashes are all distinct.
        for (i, h) in hashes.iter().enumerate() {
            let got = store.get(*h).await.expect("stored blob must be present");
            assert_eq!(got, Bytes::from(format!("blob-{i}").into_bytes()));
        }
    }

    /// The store drives correctly under Cameron's Bach simulator — the same put/get/has round-trip run
    /// on the deterministic discrete-event runtime rather than tokio. This is the seam for Bach's
    /// determinism/snapshot testing: because the trait + in-memory impl are runtime-agnostic (await-only,
    /// no tokio primitives), Bach can drive them with no changes. `.primary()` ends the sim when the task
    /// finishes; asserts inside the spawned task fail the test.
    #[test]
    fn blob_store_round_trips_under_the_bach_simulator() {
        use bach::ext::*;
        bach::sim(|| {
            async {
                let store = InMemoryBlobStore::new();
                let h = store.put(Bytes::from_static(b"deterministic")).await;
                assert_eq!(h, Hash::of(b"deterministic"));
                assert_eq!(
                    store.get(h).await,
                    Some(Bytes::from_static(b"deterministic"))
                );
                assert!(store.has(h).await);
                // genuine absence under the simulator too.
                assert_eq!(store.get(Hash::of(b"absent")).await, None);
            }
            .group("blob-store")
            .primary()
            .spawn();
        });
    }
}
