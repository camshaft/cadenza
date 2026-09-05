//! The content-addressed blob store (`design/cadenza-platform.md` §8).
//!
//! There is exactly one store: a mapping from a [`Hash`] to its bytes. Its whole interface is: put bytes
//! (getting back their hash), get bytes by hash, ask whether a hash is present, and — for the GC alone —
//! delete a hash. Everything the system keeps by hash lives here — log blobs, large state values, contract
//! declarations, wasm components — a component is not special, it is bytes addressed by its hash like any
//! other value.
//!
//! **Reference edges for GC (`design/DESIGN-cas-pinning-gc.md`).** A put declares the hashes its bytes point
//! at — `put(bytes, refs)` — so the store records the content-addressed DAG's outbound edges. The refs are
//! METADATA, not content: they do NOT affect the returned hash (still [`Hash::of`] the bytes), so two puts of
//! the same bytes are the same blob whatever refs accompany them. Recording edges here is what a later
//! reference-counting collector walks to cascade a deletion to a blob's now-unreferenced children. A leaf
//! payload passes `&[]`.
//!
//! **`delete` is privileged — GC-only.** Unlike put/get/has, [`delete`](BlobStore::delete) is not a capability
//! any reducer holds; it is the one destructive operation, reserved for the collector at a controlled
//! quiescent point (never a background sweep). It removes a hash's bytes and reports whether they were
//! present, and is idempotent — deleting an absent hash is a no-op that returns `false`.
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

/// A content-addressed blob store: hash <-> bytes. The one store of §8; backends (in-memory, disk, S3)
/// implement this and are swapped by reference. `Send + Sync` so it can be shared across the runtime's
/// concurrent tasks behind an `Arc`.
#[async_trait]
pub trait BlobStore: Send + Sync {
    /// Store `bytes` and return their content hash (tagged [`Blob`](HashTag::Blob) — the content-address
    /// kind). `refs` are the hashes these bytes point at — the blob's outbound edges in the content-addressed
    /// DAG, recorded for the reference-counting GC (`design/DESIGN-cas-pinning-gc.md`); a leaf payload passes
    /// `&[]`. Edges are METADATA, not content: they do NOT affect the returned hash, so the same bytes are the
    /// same blob whatever refs accompany them. Idempotent by construction: the hash is derived from the bytes,
    /// so putting the same bytes twice yields the same hash and simply re-stores identical content. (No
    /// `Result`: a well-formed backend's put is a pure function of its input; a fallible backend absorbs
    /// transient I/O internally, e.g. by retry — this layer stays deterministic per §8/§9.)
    async fn put(&mut self, bytes: Bytes, refs: &[Hash]) -> Hash;

    /// Fetch the bytes whose content matches `hash`, or `None` if the store does not hold them. Matching is
    /// on the [`digest`](Hash::digest) only — `hash`'s tag is ignored — so content put under one kind is
    /// fetched by a hash of any kind over the same bytes (§8). `None` is genuine absence, not a transient
    /// failure.
    async fn get(&self, hash: Hash) -> Option<Bytes>;

    /// Whether content matching `hash` (by digest, ignoring the tag) is present in the store.
    async fn has(&self, hash: Hash) -> bool;

    /// Remove the bytes matching `hash` (by digest, ignoring the tag — like [`get`](BlobStore::get)) and
    /// report whether they were present. **Privileged: the GC alone calls this** — it is the store's one
    /// destructive operation, invoked by the reference-counting collector at a controlled quiescent point
    /// (`design/DESIGN-cas-pinning-gc.md`), never by a reducer. Idempotent: deleting an absent hash is a no-op
    /// that returns `false`, so a re-run of a collection pass is safe. Removing a blob also drops its recorded
    /// [`refs`](BlobStore::put) edges.
    async fn delete(&mut self, hash: Hash) -> bool;
}

/// An in-memory [`BlobStore`] — a plain hash-map. For tests and single-process use; the smallest honest
/// backend. `put` takes `&mut self`, so no interior mutability (a lock) is needed — the runtime owns the
/// store exclusively in its event loop.
#[derive(Default)]
pub struct InMemoryBlobStore {
    /// Keyed by the content digest (the tag is not part of the store's identity — see the module docs), so
    /// the same bytes are one entry however their hash is tagged. The value carries the bytes and the blob's
    /// recorded outbound `refs` edges (declared at put, for the GC to cascade a deletion — see the module
    /// docs); a leaf blob's edge list is empty.
    blobs: HashMap<[u8; Hash::DIGEST_LEN], (Bytes, Box<[Hash]>)>,
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
        self.blobs.len()
    }

    /// Whether the store holds no blobs.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.blobs.is_empty()
    }

    /// The outbound reference edges recorded for the blob matching `hash` (by digest, ignoring the tag) at
    /// [`put`](BlobStore::put), or `None` if the store does not hold it. This is the introspection the GC's
    /// cascade reads on the in-memory backend; the trait itself stays minimal (put/get/has/delete).
    #[must_use]
    pub fn refs(&self, hash: Hash) -> Option<&[Hash]> {
        self.blobs.get(hash.digest()).map(|(_, refs)| &**refs)
    }
}

#[async_trait]
impl BlobStore for InMemoryBlobStore {
    async fn put(&mut self, bytes: Bytes, refs: &[Hash]) -> Hash {
        let hash = Hash::of(HashTag::Blob, &bytes);
        // Key on the content digest, not the tagged hash, so a lookup by any kind of hash over the same
        // bytes resolves (§8). O(1) Bytes clone into the map; the refs ride alongside as the blob's edges.
        self.blobs.insert(*hash.digest(), (bytes, Box::from(refs)));
        hash
    }

    async fn get(&self, hash: Hash) -> Option<Bytes> {
        // Match on the digest, ignoring the tag; `cloned()` on a Bytes is an O(1) refcount bump, not a copy.
        self.blobs
            .get(hash.digest())
            .map(|(bytes, _)| bytes.clone())
    }

    async fn has(&self, hash: Hash) -> bool {
        self.blobs.contains_key(hash.digest())
    }

    async fn delete(&mut self, hash: Hash) -> bool {
        // Remove by digest (like get/has) and report prior presence; dropping the entry drops its edges too.
        self.blobs.remove(hash.digest()).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::{BlobStore, InMemoryBlobStore};
    use crate::{Bytes, Hash, HashTag};

    #[tokio::test]
    async fn put_returns_the_content_hash_and_get_round_trips() {
        let mut store = InMemoryBlobStore::new();
        let bytes = Bytes::from_static(b"the hash is the capability");
        let h = store.put(bytes.clone(), &[]).await;
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
        let mut store = InMemoryBlobStore::new();
        let bytes = Bytes::from_static(b"a reducer component");
        let blob = store.put(bytes.clone(), &[]).await;
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
        let mut store = InMemoryBlobStore::new();
        let h1 = store.put(Bytes::from_static(b"same"), &[]).await;
        let h2 = store.put(Bytes::from_static(b"same"), &[]).await;
        // same bytes -> same hash, and only one blob is held.
        assert_eq!(h1, h2);
        assert_eq!(store.len(), 1);
        // distinct bytes -> a distinct hash + a second blob.
        let h3 = store.put(Bytes::from_static(b"different"), &[]).await;
        assert_ne!(h1, h3);
        assert_eq!(store.len(), 2);
    }

    #[tokio::test]
    async fn stores_and_distinguishes_many_blobs() {
        let mut store = InMemoryBlobStore::new();
        let mut hashes = Vec::new();
        for i in 0..64u16 {
            hashes.push(
                store
                    .put(Bytes::from(format!("blob-{i}").into_bytes()), &[])
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

    #[tokio::test]
    async fn put_records_refs_which_do_not_affect_the_hash() {
        // The declared outbound edges are recorded against the blob and readable back, but they are metadata:
        // the returned hash is of the bytes alone, so the same bytes with different refs are the same blob.
        let mut store = InMemoryBlobStore::new();
        let child_a = Hash::of(HashTag::Blob, b"child-a");
        let child_b = Hash::of(HashTag::Blob, b"child-b");
        let parent = store
            .put(Bytes::from_static(b"parent"), &[child_a, child_b])
            .await;
        // refs are recorded, in order, and read back by hash.
        assert_eq!(store.refs(parent), Some([child_a, child_b].as_slice()));
        // The hash is Hash::of the bytes only — refs do not enter it.
        assert_eq!(parent, Hash::of(HashTag::Blob, b"parent"));
        // Putting the SAME bytes with NO refs yields the SAME hash (edges are not content); it is one blob,
        // and the last put's edges win (a producer disagreeing about a blob's refs is a producer bug).
        let again = store.put(Bytes::from_static(b"parent"), &[]).await;
        assert_eq!(again, parent);
        assert_eq!(store.len(), 1);
        assert_eq!(store.refs(parent), Some([].as_slice()));
        // A leaf blob (no refs) records an empty edge list, not `None` (absent).
        let leaf = store.put(Bytes::from_static(b"leaf"), &[]).await;
        assert_eq!(store.refs(leaf), Some([].as_slice()));
        // An absent hash has no recorded refs.
        assert_eq!(store.refs(Hash::of(HashTag::Blob, b"never")), None);
    }

    #[tokio::test]
    async fn delete_removes_is_idempotent_and_re_put_restores() {
        let mut store = InMemoryBlobStore::new();
        let child = Hash::of(HashTag::Blob, b"child");
        let h = store.put(Bytes::from_static(b"doomed"), &[child]).await;
        assert!(store.has(h).await);
        // delete reports prior presence, drops the bytes AND the recorded edges.
        assert!(
            store.delete(h).await,
            "delete of a present blob returns true"
        );
        assert!(!store.has(h).await);
        assert_eq!(store.get(h).await, None);
        assert_eq!(store.refs(h), None, "delete drops the blob's edges too");
        assert_eq!(store.len(), 0);
        // Idempotent: deleting an already-absent hash is a no-op returning false.
        assert!(
            !store.delete(h).await,
            "delete of an absent blob returns false"
        );
        // delete matches by digest, ignoring the tag (like get/has): a same-digest Program hash also deletes.
        let h2 = store.put(Bytes::from_static(b"tagged"), &[]).await;
        let program = Hash::of(HashTag::Program, b"tagged");
        assert!(
            store.delete(program).await,
            "delete matches on digest, not tag"
        );
        assert!(!store.has(h2).await);
        // Re-put restores the content (content-addressing: the same bytes return the same hash).
        let restored = store.put(Bytes::from_static(b"doomed"), &[child]).await;
        assert_eq!(restored, h);
        assert!(store.has(h).await);
        assert_eq!(store.refs(h), Some([child].as_slice()));
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
                let mut store = InMemoryBlobStore::new();
                let h = store.put(Bytes::from_static(b"deterministic"), &[]).await;
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
