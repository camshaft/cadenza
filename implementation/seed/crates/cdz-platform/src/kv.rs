//! The key-value store (`design/cadenza-platform.md` §7).
//!
//! A reducer's state is a key-value store, defined by this interface rather than a concrete in-memory
//! structure, so the backend is pluggable (in memory for tests, a structurally-shared persistent map, or
//! disk/network for state too large for RAM) while the reducer only ever sees the trait. This first slice
//! is the core map: get, put, delete. The two richer §7 obligations — a streaming prefix-scan over a
//! canonical key order, and a content-addressed root hash after each change — are a later slice, added
//! when the reducer layer needs them; the interface is designed to grow into them.
//!
//! Keys and values are both [`Bytes`] — the platform marshals everything as bytes, and byte keys order
//! lexicographically, which is the canonical order the future prefix-scan will walk. Small values live
//! inline here; large values (transcripts, model payloads) are stored in the blob store and held as a
//! [`Hash`](crate::Hash), so the value bytes are just that hash.
//!
//! The operations are async (like the blob store) so a disk- or network-backed store fetches without
//! blocking, while `get` stays deterministic — a pure function of the key against the current state,
//! regardless of backend latency (§7). The trait is [`async_trait`] so a backend is a dyn-safe swappable
//! trait object, and the methods are runtime-agnostic (they only await), so they drive under tokio in
//! production and under the Bach simulator in deterministic tests alike.

use crate::Bytes;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;

/// A key-value store: `Bytes` keys to `Bytes` values (§7). Backends (in memory, persistent, disk) implement
/// this and are swapped by reference. `Send + Sync` so it can be shared across the runtime's concurrent
/// tasks behind an `Arc`.
#[async_trait]
pub trait KvStore: Send + Sync {
    /// The value stored under `key`, or `None` if the store holds no entry for it.
    async fn get(&self, key: &[u8]) -> Option<Bytes>;

    /// Insert or overwrite the value under `key`. Last write wins, as a map does.
    async fn put(&self, key: Bytes, value: Bytes);

    /// Remove the entry under `key`, returning `true` if one was present (and is now gone), `false` if
    /// there was nothing to remove.
    async fn delete(&self, key: &[u8]) -> bool;
}

/// An in-memory [`KvStore`] — a plain hash-map behind a `Mutex`. For tests and single-process use; the
/// smallest honest backend. Runtime-agnostic: the lock is a `std::sync::Mutex` held only across the map
/// operation (never across an `.await`), so this drives identically under tokio and under Bach.
#[derive(Default)]
pub struct InMemoryKvStore {
    entries: Mutex<HashMap<Bytes, Bytes>>,
}

impl InMemoryKvStore {
    /// An empty in-memory store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of entries held. Handy for tests and introspection.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.lock().expect("kv-store mutex poisoned").len()
    }

    /// Whether the store holds no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries
            .lock()
            .expect("kv-store mutex poisoned")
            .is_empty()
    }
}

#[async_trait]
impl KvStore for InMemoryKvStore {
    async fn get(&self, key: &[u8]) -> Option<Bytes> {
        // `cloned()` on a Bytes is an O(1) refcount bump, not a copy. `Bytes: Borrow<[u8]>` lets a byte
        // slice look up a Bytes-keyed map without allocating a key.
        self.entries
            .lock()
            .expect("kv-store mutex poisoned")
            .get(key)
            .cloned()
    }

    async fn put(&self, key: Bytes, value: Bytes) {
        self.entries
            .lock()
            .expect("kv-store mutex poisoned")
            .insert(key, value);
    }

    async fn delete(&self, key: &[u8]) -> bool {
        self.entries
            .lock()
            .expect("kv-store mutex poisoned")
            .remove(key)
            .is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::{InMemoryKvStore, KvStore};
    use crate::Bytes;

    #[tokio::test]
    async fn put_then_get_round_trips() {
        let kv = InMemoryKvStore::new();
        kv.put(
            Bytes::from_static(b"tick/interval-ms"),
            Bytes::from_static(b"900000"),
        )
        .await;
        assert_eq!(
            kv.get(b"tick/interval-ms").await,
            Some(Bytes::from_static(b"900000"))
        );
    }

    #[tokio::test]
    async fn get_reports_absence() {
        let kv = InMemoryKvStore::new();
        assert_eq!(kv.get(b"missing").await, None);
    }

    #[tokio::test]
    async fn put_overwrites_last_write_wins() {
        let kv = InMemoryKvStore::new();
        kv.put(Bytes::from_static(b"k"), Bytes::from_static(b"first"))
            .await;
        kv.put(Bytes::from_static(b"k"), Bytes::from_static(b"second"))
            .await;
        assert_eq!(kv.get(b"k").await, Some(Bytes::from_static(b"second")));
        // an overwrite is not a second entry.
        assert_eq!(kv.len(), 1);
    }

    #[tokio::test]
    async fn delete_removes_and_reports_presence() {
        let kv = InMemoryKvStore::new();
        kv.put(Bytes::from_static(b"k"), Bytes::from_static(b"v"))
            .await;
        assert!(kv.delete(b"k").await, "deleting a present key returns true");
        assert_eq!(kv.get(b"k").await, None);
        assert!(kv.is_empty());
        // deleting an absent key returns false.
        assert!(!kv.delete(b"k").await);
    }

    #[tokio::test]
    async fn holds_many_distinct_keys() {
        let kv = InMemoryKvStore::new();
        for i in 0..64u16 {
            kv.put(
                Bytes::from(format!("key-{i}").into_bytes()),
                Bytes::from(format!("val-{i}").into_bytes()),
            )
            .await;
        }
        assert_eq!(kv.len(), 64);
        for i in 0..64u16 {
            let got = kv.get(format!("key-{i}").as_bytes()).await;
            assert_eq!(got, Some(Bytes::from(format!("val-{i}").into_bytes())));
        }
    }

    /// The store drives correctly under Cameron's Bach simulator — the same put/get/delete run on the
    /// deterministic discrete-event runtime rather than tokio. Because the trait and in-memory impl are
    /// runtime-agnostic (await-only, no tokio primitives), Bach drives them unchanged — the seam for
    /// determinism and snapshot testing. `.primary()` ends the sim when the task finishes; asserts inside
    /// the spawned task fail the test.
    #[test]
    fn kv_store_round_trips_under_the_bach_simulator() {
        use bach::ext::*;
        bach::sim(|| {
            async {
                let kv = InMemoryKvStore::new();
                kv.put(Bytes::from_static(b"seen/msg-1"), Bytes::from_static(b"1"))
                    .await;
                assert_eq!(kv.get(b"seen/msg-1").await, Some(Bytes::from_static(b"1")));
                assert!(kv.delete(b"seen/msg-1").await);
                assert_eq!(kv.get(b"seen/msg-1").await, None);
            }
            .group("kv-store")
            .primary()
            .spawn();
        });
    }
}
