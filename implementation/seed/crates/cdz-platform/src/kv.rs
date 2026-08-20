//! The key-value store (`design/cadenza-platform.md` §7).
//!
//! A reducer's state is a key-value store, defined by this interface rather than a concrete in-memory
//! structure, so the backend is pluggable (in memory for tests, a structurally-shared persistent map, or
//! disk/network for state too large for RAM) while the reducer only ever sees the trait. The operations
//! are get, put, delete, and a streaming prefix-scan over the canonical key order — the primitive for the
//! collections a reducer maintains (pending children, seen items, per-target working state). The remaining
//! §7 obligation, a content-addressed root hash after each change, is a later slice.
//!
//! Keys and values are both [`Bytes`], and keys order lexicographically — the canonical key order the
//! prefix-scan walks, and why the in-memory backend is a `BTreeMap` (sorted) rather than a hash map. Small
//! values live inline here; large values (transcripts, model payloads) are stored in the blob store and
//! held as a [`Hash`](crate::Hash), so the value bytes are just that hash.
//!
//! The operations are async (like the blob store) so a disk- or network-backed store fetches without
//! blocking, while `get` stays deterministic — a pure function of the key against the current state (§7).
//! `scan_prefix` returns a lazy [`Stream`] rather than a materialized collection, so a large scan does not
//! load every matching entry at once (a disk/network backend pages it as the consumer pulls). The trait is
//! [`async_trait`] so a backend is a dyn-safe swappable trait object, and the methods are runtime-agnostic
//! (they only await), so they drive under tokio in production and under the Bach simulator in deterministic
//! tests alike.

use crate::Bytes;
use async_trait::async_trait;
use futures_core::Stream;
use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::Mutex;

/// A prefix-scan result: a lazy stream of `(key, value)` pairs in ascending key order. Boxed so the
/// [`KvStore`] trait stays object-safe; `Send` so it can cross the runtime's tasks. The lifetime ties the
/// stream to the store it was opened on (an in-memory backend returns an owned, `'static` snapshot stream,
/// which satisfies any `'a`).
pub type KvScan<'a> = Pin<Box<dyn Stream<Item = (Bytes, Bytes)> + Send + 'a>>;

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

    /// Stream every `(key, value)` whose key begins with `prefix`, in ascending key order. This is a lazy
    /// [`Stream`], not a materialized collection, so a scan that matches a great many keys does not load
    /// them all at once — a disk/network backend pages entries as the consumer pulls. An empty `prefix`
    /// scans the whole store.
    fn scan_prefix(&self, prefix: &[u8]) -> KvScan<'_>;
}

/// An in-memory [`KvStore`] — a `BTreeMap` behind a `Mutex`. For tests and single-process use; the smallest
/// honest backend. A `BTreeMap` (not a hash map) keeps keys in the canonical ascending order the
/// prefix-scan needs. Runtime-agnostic: the lock is a `std::sync::Mutex` held only across the map operation
/// (never across an `.await`), so this drives identically under tokio and under Bach.
#[derive(Default)]
pub struct InMemoryKvStore {
    entries: Mutex<BTreeMap<Bytes, Bytes>>,
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

    fn scan_prefix(&self, prefix: &[u8]) -> KvScan<'_> {
        // The trait promises a lazy stream so a disk/network backend can page a huge scan. An in-memory
        // backend already holds its data resident, so it snapshots the matching range under the lock —
        // the pairs are O(1) Bytes-refcount clones, and only the entries that MATCH the prefix are taken
        // (the BTreeMap range stops at the first key past the prefix) — then streams that snapshot. This
        // keeps the lock held only briefly (never across the consumer's awaits) while honoring the lazy
        // Stream contract at the trait boundary.
        let matches: Vec<(Bytes, Bytes)> = {
            use std::ops::Bound;
            let guard = self.entries.lock().expect("kv-store mutex poisoned");
            // Range from the prefix (inclusive) to the end, then take only while the key still carries
            // the prefix — so the walk stops at the first key past it, touching just the matches.
            guard
                .range::<[u8], _>((Bound::Included(prefix), Bound::Unbounded))
                .take_while(|(k, _)| k.starts_with(prefix))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        };
        Box::pin(futures_util::stream::iter(matches))
    }
}

#[cfg(test)]
mod tests {
    use super::{InMemoryKvStore, KvStore};
    use crate::Bytes;
    use futures_util::StreamExt;

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
        assert!(!kv.delete(b"k").await);
    }

    #[tokio::test]
    async fn scan_prefix_streams_matching_keys_in_order() {
        let kv = InMemoryKvStore::new();
        // insert out of order + across two prefixes; the scan must return only the prefix, sorted.
        for (k, v) in [
            ("seen/c", "3"),
            ("seen/a", "1"),
            ("tick/x", "9"),
            ("seen/b", "2"),
        ] {
            kv.put(
                Bytes::from(k.as_bytes().to_vec()),
                Bytes::from(v.as_bytes().to_vec()),
            )
            .await;
        }
        let got: Vec<(Bytes, Bytes)> = kv.scan_prefix(b"seen/").collect().await;
        assert_eq!(
            got,
            vec![
                (Bytes::from_static(b"seen/a"), Bytes::from_static(b"1")),
                (Bytes::from_static(b"seen/b"), Bytes::from_static(b"2")),
                (Bytes::from_static(b"seen/c"), Bytes::from_static(b"3")),
            ],
            "scan_prefix yields only the prefix's keys, ascending"
        );
    }

    #[tokio::test]
    async fn scan_prefix_edges_empty_and_all() {
        let kv = InMemoryKvStore::new();
        // empty store -> empty scan.
        assert_eq!(kv.scan_prefix(b"any").count().await, 0);
        kv.put(Bytes::from_static(b"a"), Bytes::from_static(b"1"))
            .await;
        kv.put(Bytes::from_static(b"b"), Bytes::from_static(b"2"))
            .await;
        // an empty prefix scans the whole store, in order.
        let all: Vec<Bytes> = kv.scan_prefix(b"").map(|(k, _)| k).collect().await;
        assert_eq!(
            all,
            vec![Bytes::from_static(b"a"), Bytes::from_static(b"b")]
        );
        // a prefix matching nothing -> empty; a prefix that is a whole key -> just it.
        assert_eq!(kv.scan_prefix(b"z").count().await, 0);
        assert_eq!(kv.scan_prefix(b"a").count().await, 1);
    }

    #[tokio::test]
    async fn holds_many_distinct_keys() {
        let kv = InMemoryKvStore::new();
        for i in 0..64u16 {
            kv.put(
                Bytes::from(format!("key-{i:02}").into_bytes()),
                Bytes::from(format!("val-{i}").into_bytes()),
            )
            .await;
        }
        assert_eq!(kv.len(), 64);
        for i in 0..64u16 {
            let got = kv.get(format!("key-{i:02}").as_bytes()).await;
            assert_eq!(got, Some(Bytes::from(format!("val-{i}").into_bytes())));
        }
    }

    /// The store drives correctly under Cameron's Bach simulator — put/get/delete and a prefix-scan run on
    /// the deterministic discrete-event runtime rather than tokio. Because the trait and in-memory impl are
    /// runtime-agnostic (await-only, no tokio primitives; the scan is a snapshot stream), Bach drives them
    /// unchanged — the seam for determinism and snapshot testing. `.primary()` ends the sim when the task
    /// finishes; asserts inside the spawned task fail the test.
    #[test]
    fn kv_store_round_trips_under_the_bach_simulator() {
        use bach::ext::*;
        use futures_util::StreamExt;
        bach::sim(|| {
            async {
                let kv = InMemoryKvStore::new();
                kv.put(Bytes::from_static(b"seen/msg-1"), Bytes::from_static(b"1"))
                    .await;
                kv.put(Bytes::from_static(b"seen/msg-2"), Bytes::from_static(b"2"))
                    .await;
                assert_eq!(kv.get(b"seen/msg-1").await, Some(Bytes::from_static(b"1")));
                assert_eq!(kv.scan_prefix(b"seen/").count().await, 2);
                assert!(kv.delete(b"seen/msg-1").await);
                assert_eq!(kv.scan_prefix(b"seen/").count().await, 1);
            }
            .group("kv-store")
            .primary()
            .spawn();
        });
    }
}
