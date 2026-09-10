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

/// Why a [`BlobStore`] operation failed. **Absence is NOT an error** — [`get`](BlobStore::get) returns
/// `Ok(None)` for a genuine miss; an `Err` means the store could not DETERMINE the answer (a transport/I-O
/// failure, a rejected credential, or bytes that don't match their hash). A single CONCRETE type (not an
/// associated `type Error`) so `dyn BlobStore` stays object-safe; each backend maps its own failure into a
/// variant.
#[derive(Debug, Clone)]
pub enum BlobStoreError {
    /// A transport / I-O failure talking to the backend (disk, network, S3) — possibly transient.
    Io(String),
    /// A credential was missing or rejected by the backend.
    Unauthorized,
    /// The backend returned bytes that do not match the requested hash — a content-address violation. The
    /// bytes are discarded (you cannot forge bytes for a hash).
    Corrupt(String),
}

impl std::fmt::Display for BlobStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(msg) => write!(f, "blob store I/O error: {msg}"),
            Self::Unauthorized => {
                write!(f, "blob store unauthorized: credential missing or rejected")
            }
            Self::Corrupt(msg) => write!(f, "blob store content-address violation: {msg}"),
        }
    }
}

impl std::error::Error for BlobStoreError {}

/// A content-addressed blob store: hash <-> bytes. The one store of §8; backends (in-memory, disk, S3)
/// implement this and are swapped by reference. `Send + Sync` so it can be shared across the runtime's
/// concurrent tasks behind an `Arc`.
#[async_trait]
pub trait BlobStore: Send + Sync {
    /// Store `bytes` and return their content hash (tagged [`Blob`](HashTag::Blob) — the content-address
    /// kind). Idempotent by construction: the hash is derived from the bytes, so putting the same bytes twice
    /// yields the same hash and simply re-stores identical content. Returns a [`BlobStoreError`] if the
    /// backend could not persist the bytes (a real disk/network/S3 backend genuinely fails — the caller,
    /// e.g. an HTTP CAS server, needs to know rather than silently drop the write).
    ///
    /// Takes `&self`: a backend uses INTERIOR MUTABILITY for its writable state so a shared store (behind an
    /// `Arc`) can serve concurrent `put`/`get`/`has` without an external lock. `&mut self` would force every
    /// caller — a concurrent HTTP CAS server, say — to wrap the store in a `Mutex` that falsely serializes
    /// even reads.
    async fn put(&self, bytes: Bytes) -> Result<Hash, BlobStoreError>;

    /// Fetch the bytes whose content matches `hash`, or `Ok(None)` if the store genuinely does not hold
    /// them. Matching is on the [`digest`](Hash::digest) only — `hash`'s tag is ignored — so content put
    /// under one kind is fetched by a hash of any kind over the same bytes (§8). `Ok(None)` is genuine
    /// absence; an `Err` means the store could not determine whether it holds the bytes (transport/I-O/auth).
    async fn get(&self, hash: Hash) -> Result<Option<Bytes>, BlobStoreError>;

    /// Whether content matching `hash` (by digest, ignoring the tag) is present. `Err` on a failure to
    /// determine it (transport/I-O/auth).
    async fn has(&self, hash: Hash) -> Result<bool, BlobStoreError>;
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
    // An in-memory map never fails (a poisoned lock is a bug, not a store error), so every op is `Ok`.
    async fn put(&self, bytes: Bytes) -> Result<Hash, BlobStoreError> {
        let hash = Hash::of(HashTag::Blob, &bytes);
        // Key on the content digest, not the tagged hash, so a lookup by any kind of hash over the same
        // bytes resolves (§8). O(1) Bytes clone into the map, under a brief write lock.
        self.blobs
            .write()
            .expect("blob store lock not poisoned")
            .insert(*hash.digest(), bytes);
        Ok(hash)
    }

    async fn get(&self, hash: Hash) -> Result<Option<Bytes>, BlobStoreError> {
        // Match on the digest, ignoring the tag; `cloned()` on a Bytes is an O(1) refcount bump, not a copy.
        // A read lock, so concurrent gets don't block each other.
        Ok(self
            .blobs
            .read()
            .expect("blob store lock not poisoned")
            .get(hash.digest())
            .cloned())
    }

    async fn has(&self, hash: Hash) -> Result<bool, BlobStoreError> {
        Ok(self
            .blobs
            .read()
            .expect("blob store lock not poisoned")
            .contains_key(hash.digest()))
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
        let h = store.put(bytes.clone()).await.unwrap();
        // put returns the content hash of exactly those bytes.
        assert_eq!(h, Hash::of(HashTag::Blob, &bytes));
        // get by that hash returns the same bytes.
        assert_eq!(store.get(h).await.unwrap(), Some(bytes));
        assert!(store.has(h).await.unwrap());
    }

    #[tokio::test]
    async fn a_hash_of_any_kind_over_the_same_bytes_resolves() {
        // The store keys on content, not kind: bytes put here (returning a Blob hash) are fetched equally by
        // a Program hash over the same bytes — the addressing the wasm program store relies on (§8).
        let store = InMemoryBlobStore::new();
        let bytes = Bytes::from_static(b"a reducer component");
        let blob = store.put(bytes.clone()).await.unwrap();
        let program = Hash::of(HashTag::Program, &bytes); // same digest, different (Program) tag
        assert_ne!(
            blob, program,
            "the two hashes differ (their tag bytes differ)"
        );
        assert_eq!(blob.digest(), program.digest());
        assert_eq!(
            store.get(program).await.unwrap(),
            Some(bytes),
            "fetched by the Program-tagged hash"
        );
        assert!(store.has(program).await.unwrap());
    }

    #[tokio::test]
    async fn get_and_has_report_absence() {
        let store = InMemoryBlobStore::new();
        let absent = Hash::of(HashTag::Blob, b"never stored");
        assert_eq!(store.get(absent).await.unwrap(), None);
        assert!(!store.has(absent).await.unwrap());
    }

    #[tokio::test]
    async fn put_is_idempotent_by_content() {
        let store = InMemoryBlobStore::new();
        let h1 = store.put(Bytes::from_static(b"same")).await.unwrap();
        let h2 = store.put(Bytes::from_static(b"same")).await.unwrap();
        // same bytes -> same hash, and only one blob is held.
        assert_eq!(h1, h2);
        assert_eq!(store.len(), 1);
        // distinct bytes -> a distinct hash + a second blob.
        let h3 = store.put(Bytes::from_static(b"different")).await.unwrap();
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
                    .await
                    .unwrap(),
            );
        }
        assert_eq!(store.len(), 64);
        // every stored blob is retrievable and the hashes are all distinct.
        for (i, h) in hashes.iter().enumerate() {
            let got = store
                .get(*h)
                .await
                .unwrap()
                .expect("stored blob must be present");
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
                let h = store
                    .put(Bytes::from_static(b"deterministic"))
                    .await
                    .unwrap();
                assert_eq!(h, Hash::of(HashTag::Blob, b"deterministic"));
                assert_eq!(
                    store.get(h).await.unwrap(),
                    Some(Bytes::from_static(b"deterministic"))
                );
                assert!(store.has(h).await.unwrap());
                // genuine absence under the simulator too.
                assert_eq!(
                    store.get(Hash::of(HashTag::Blob, b"absent")).await.unwrap(),
                    None
                );
            }
            .group("blob-store")
            .primary()
            .spawn();
        });
    }
}
