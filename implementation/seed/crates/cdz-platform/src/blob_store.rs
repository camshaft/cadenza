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
//! **Keyed on content, not kind.** The store addresses by the [`digest`](Hash::digest) — the content
//! commitment — and ignores the leading [`HashTag`] byte. The tag says what a hash *names* (a program, a
//! blob, a contract); it is a typed *view* on the same content, not part of the store's identity. So the
//! same bytes stored once are reachable by any hash over them, whatever its kind: a wasm component put here
//! is fetched equally by its [`Blob`](HashTag::Blob) hash or by the [`Program`](crate::ProgramHash) hash
//! that names it as a program. The digest is the capability; the kind is the caller's interpretation.
//!
//! The operations are **async** so a disk/network-backed store (a local cache, S3) can fetch without
//! blocking the runtime — but they stay deterministic: `get(hash)` is a pure function of the hash
//! (content-addressed, the same bytes every time) and `put(bytes)` a pure function of the bytes, so
//! awaiting a fetch changes only timing, never the result. The trait is [`async_trait`] so a backend is a
//! dyn-safe swappable trait object, and the methods are runtime-agnostic (they only await), so they run
//! under tokio in production and under the Bach simulator in deterministic tests alike.

use crate::{Bytes, Hash, HashTag};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::RwLock;

/// A content-addressed blob store: hash <-> bytes. The one store of §8; backends (in-memory, disk, S3)
/// implement this and are swapped by reference. `Send + Sync` so it can be shared across the runtime's
/// concurrent tasks behind an `Arc`.
#[async_trait]
pub trait BlobStore: Send + Sync {
    /// Store `bytes` and return their content hash (tagged [`Blob`](HashTag::Blob) — the content-address
    /// kind). Idempotent by construction: the hash is derived from the bytes, so putting the same bytes twice
    /// yields the same hash and simply re-stores identical content. (No `Result`: a well-formed backend's put
    /// is a pure function of its input; a fallible backend absorbs transient I/O internally, e.g. by retry —
    /// this layer stays deterministic per §8/§9.)
    ///
    /// Takes `&self`: a backend uses INTERIOR MUTABILITY for its writable state so a shared store (behind an
    /// `Arc`) can serve concurrent `put`/`get`/`has` without an external lock. `&mut self` would force every
    /// caller — a concurrent HTTP CAS server, say — to wrap the store in a `Mutex` that falsely serializes
    /// even reads.
    async fn put(&self, bytes: Bytes) -> Hash;

    /// Fetch the bytes whose content matches `hash`, or `None` if the store does not hold them. Matching is
    /// on the [`digest`](Hash::digest) only — `hash`'s tag is ignored — so content put under one kind is
    /// fetched by a hash of any kind over the same bytes (§8). `None` is genuine absence, not a transient
    /// failure.
    async fn get(&self, hash: Hash) -> Option<Bytes>;

    /// Whether content matching `hash` (by digest, ignoring the tag) is present in the store.
    async fn has(&self, hash: Hash) -> bool;
}

/// An in-memory [`BlobStore`] — a hash-map behind a `RwLock` for interior mutability. For tests and
/// single-process use; the smallest honest backend. The `RwLock` lets a shared store (behind an `Arc`)
/// serve concurrent reads (`get`/`has` take a read lock) while `put` takes a brief write lock — so a
/// concurrent server needs no external mutex. Every critical section is a plain `HashMap` op with no
/// `await` held across the lock, so it never blocks the async runtime.
#[derive(Default)]
pub struct InMemoryBlobStore {
    /// Keyed by the content digest (the tag is not part of the store's identity — see the module docs), so
    /// the same bytes are one entry however their hash is tagged.
    blobs: RwLock<HashMap<[u8; Hash::DIGEST_LEN], Bytes>>,
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
        self.blobs
            .read()
            .expect("blob store lock not poisoned")
            .len()
    }

    /// Whether the store holds no blobs.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.blobs
            .read()
            .expect("blob store lock not poisoned")
            .is_empty()
    }
}

#[async_trait]
impl BlobStore for InMemoryBlobStore {
    async fn put(&self, bytes: Bytes) -> Hash {
        let hash = Hash::of(HashTag::Blob, &bytes);
        // Key on the content digest, not the tagged hash, so a lookup by any kind of hash over the same
        // bytes resolves (§8). O(1) Bytes clone into the map, under a brief write lock.
        self.blobs
            .write()
            .expect("blob store lock not poisoned")
            .insert(*hash.digest(), bytes);
        hash
    }

    async fn get(&self, hash: Hash) -> Option<Bytes> {
        // Match on the digest, ignoring the tag; `cloned()` on a Bytes is an O(1) refcount bump, not a copy.
        // A read lock, so concurrent gets don't block each other.
        self.blobs
            .read()
            .expect("blob store lock not poisoned")
            .get(hash.digest())
            .cloned()
    }

    async fn has(&self, hash: Hash) -> bool {
        self.blobs
            .read()
            .expect("blob store lock not poisoned")
            .contains_key(hash.digest())
    }
}

#[cfg(test)]
mod tests {
    use super::{BlobStore, InMemoryBlobStore};
    use crate::{Bytes, Hash, HashTag};

    #[tokio::test]
    async fn put_returns_the_content_hash_and_get_round_trips() {
        let store = InMemoryBlobStore::new();
        let bytes = Bytes::from_static(b"the hash is the capability");
        let h = store.put(bytes.clone()).await;
        // put returns the content hash of exactly those bytes.
        assert_eq!(h, Hash::of(HashTag::Blob, &bytes));
        // get by that hash returns the same bytes.
        assert_eq!(store.get(h).await, Some(bytes));
        assert!(store.has(h).await);
    }

    #[tokio::test]
    async fn a_hash_of_any_kind_over_the_same_bytes_resolves() {
        // The store keys on content, not kind: bytes put here (returning a Blob hash) are fetched equally by
        // a Program hash over the same bytes — the addressing the wasm program store relies on (§8).
        let store = InMemoryBlobStore::new();
        let bytes = Bytes::from_static(b"a reducer component");
        let blob = store.put(bytes.clone()).await;
        let program = Hash::of(HashTag::Program, &bytes); // same digest, different (Program) tag
        assert_ne!(
            blob, program,
            "the two hashes differ (their tag bytes differ)"
        );
        assert_eq!(blob.digest(), program.digest());
        assert_eq!(
            store.get(program).await,
            Some(bytes),
            "fetched by the Program-tagged hash"
        );
        assert!(store.has(program).await);
    }

    #[tokio::test]
    async fn get_and_has_report_absence() {
        let store = InMemoryBlobStore::new();
        let absent = Hash::of(HashTag::Blob, b"never stored");
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
                assert_eq!(h, Hash::of(HashTag::Blob, b"deterministic"));
                assert_eq!(
                    store.get(h).await,
                    Some(Bytes::from_static(b"deterministic"))
                );
                assert!(store.has(h).await);
                // genuine absence under the simulator too.
                assert_eq!(store.get(Hash::of(HashTag::Blob, b"absent")).await, None);
            }
            .group("blob-store")
            .primary()
            .spawn();
        });
    }
}
