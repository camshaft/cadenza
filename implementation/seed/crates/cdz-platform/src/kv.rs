//! The key-value store (`design/cadenza-platform.md` §7).
//!
//! A reducer's state is a key-value store, defined by this interface rather than a concrete in-memory
//! structure, so the backend is pluggable (in memory for tests, a structurally-shared persistent map, or
//! disk/network for state too large for RAM) while the reducer only ever sees the trait. The operations
//! are get, put, delete, and a streaming range-scan over the canonical key order (with a keys-only
//! variant) — the primitive for the collections a reducer maintains (pending children, seen items,
//! per-target working state). A prefix scan is just a range scan over the prefix's key range, via the
//! [`prefix_range`] helper. The remaining §7 obligation, a content-addressed root hash after each change,
//! is a later slice.
//!
//! Keys and values are both [`Bytes`], and keys order lexicographically — the canonical key order the
//! scan walks, and why the in-memory backend is a `BTreeMap` (sorted) rather than a hash map. Small values
//! live inline here; large values (transcripts, model payloads) are stored in the blob store and held as a
//! [`Hash`](crate::Hash), so the value bytes are just that hash.
//!
//! The operations are async (like the blob store) so a disk- or network-backed store fetches without
//! blocking, while `get` stays deterministic — a pure function of the key against the current state (§7).
//! The scans return a lazy [`Stream`] rather than a materialized collection, so a large scan does not load
//! every matching entry at once (a disk/network backend pages it as the consumer pulls). The trait is
//! [`async_trait`] so a backend is a dyn-safe swappable trait object, and the methods are runtime-agnostic
//! (they only await), so they drive under tokio in production and under the Bach simulator in deterministic
//! tests alike.

use crate::Bytes;
use async_trait::async_trait;
use futures_core::Stream;
use std::collections::BTreeMap;
use std::ops::Bound;
use std::pin::Pin;
use std::sync::Mutex;

/// A key range over the canonical (lexicographic) key order — the same shape `BTreeMap::range` takes, but
/// a concrete owned type (not a generic `RangeBounds`) because the [`KvStore`] scan methods are on a
/// dyn-safe trait, which cannot be generic. Build one directly, or with [`prefix_range`] for the common
/// prefix case. `Bytes` bounds clone in O(1).
pub type KeyRange = (Bound<Bytes>, Bound<Bytes>);

/// A range-scan of `(key, value)` pairs: a lazy stream in ascending key order. Boxed so the [`KvStore`]
/// trait stays object-safe; `Send` so it can cross the runtime's tasks. An in-memory backend returns an
/// owned, `'static` snapshot stream, which satisfies any `'a`.
pub type KvScan<'a> = Pin<Box<dyn Stream<Item = (Bytes, Bytes)> + Send + 'a>>;

/// A range-scan of keys only — same as [`KvScan`] but the stream yields just the keys (no value bytes),
/// for when the caller only needs to enumerate keys.
pub type KvKeyScan<'a> = Pin<Box<dyn Stream<Item = Bytes> + Send + 'a>>;

/// The key range that matches exactly the keys beginning with `prefix`: `[prefix, successor(prefix))`,
/// where the successor increments the last byte below `0xFF` and drops trailing `0xFF` bytes. An empty
/// prefix, or an all-`0xFF` prefix, has no finite successor, so the upper bound is unbounded (the scan
/// runs to the end). This is the prefix scan expressed as a range.
#[must_use]
pub fn prefix_range(prefix: &[u8]) -> KeyRange {
    let start = Bound::Included(Bytes::copy_from_slice(prefix));
    let mut end = prefix.to_vec();
    while let Some(&last) = end.last() {
        if last < 0xFF {
            *end.last_mut().expect("non-empty: just checked last()") = last + 1;
            return (start, Bound::Excluded(Bytes::from(end)));
        }
        end.pop(); // trailing 0xFF: it cannot be incremented, so drop it and carry.
    }
    (start, Bound::Unbounded)
}

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

    /// Stream every `(key, value)` whose key falls in `range`, in ascending key order. A lazy [`Stream`],
    /// not a materialized collection, so a scan that matches a great many keys does not load them all at
    /// once. Use [`prefix_range`] for a prefix scan, or `(Bound::Unbounded, Bound::Unbounded)` for the
    /// whole store.
    fn scan(&self, range: KeyRange) -> KvScan<'_>;

    /// Like [`scan`](KvStore::scan) but the stream yields only the keys in `range` — for when the caller
    /// enumerates keys without needing the value bytes.
    fn scan_keys(&self, range: KeyRange) -> KvKeyScan<'_>;
}

/// An in-memory [`KvStore`] — a `BTreeMap` behind a `Mutex`. For tests and single-process use; the smallest
/// honest backend. A `BTreeMap` (not a hash map) keeps keys in the canonical ascending order the scans
/// need. Runtime-agnostic: the lock is a `std::sync::Mutex` held only across the map operation (never
/// across an `.await`), so this drives identically under tokio and under Bach.
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

    fn scan(&self, range: KeyRange) -> KvScan<'_> {
        // The trait promises a lazy stream so a disk/network backend can page a huge scan. An in-memory
        // backend already holds its data resident, so it snapshots the requested range under the lock —
        // the pairs are O(1) Bytes-refcount clones, and BTreeMap::range visits only the keys in range —
        // then streams that snapshot, so the lock is never held across the consumer's awaits.
        let pairs: Vec<(Bytes, Bytes)> = {
            let guard = self.entries.lock().expect("kv-store mutex poisoned");
            guard
                .range(range)
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        };
        Box::pin(futures_util::stream::iter(pairs))
    }

    fn scan_keys(&self, range: KeyRange) -> KvKeyScan<'_> {
        let keys: Vec<Bytes> = {
            let guard = self.entries.lock().expect("kv-store mutex poisoned");
            guard.range(range).map(|(k, _)| k.clone()).collect()
        };
        Box::pin(futures_util::stream::iter(keys))
    }
}

#[cfg(test)]
mod tests {
    use super::{InMemoryKvStore, KeyRange, KvStore, prefix_range};
    use crate::Bytes;
    use futures_util::StreamExt;
    use std::ops::Bound;

    /// The whole-store range.
    fn all() -> KeyRange {
        (Bound::Unbounded, Bound::Unbounded)
    }

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
    async fn scan_streams_a_range_of_pairs_ascending() {
        let kv = InMemoryKvStore::new();
        for (k, v) in [("c", "3"), ("a", "1"), ("e", "5"), ("b", "2"), ("d", "4")] {
            kv.put(
                Bytes::from(k.as_bytes().to_vec()),
                Bytes::from(v.as_bytes().to_vec()),
            )
            .await;
        }
        // half-open range [b, d): b, c — sorted, excluding d.
        let range = (
            Bound::Included(Bytes::from_static(b"b")),
            Bound::Excluded(Bytes::from_static(b"d")),
        );
        let got: Vec<(Bytes, Bytes)> = kv.scan(range).collect().await;
        assert_eq!(
            got,
            vec![
                (Bytes::from_static(b"b"), Bytes::from_static(b"2")),
                (Bytes::from_static(b"c"), Bytes::from_static(b"3")),
            ]
        );
        // whole-store scan is everything, ascending.
        let all_keys: Vec<Bytes> = kv.scan(all()).map(|(k, _)| k).collect().await;
        assert_eq!(
            all_keys,
            [b"a", b"b", b"c", b"d", b"e"]
                .map(|k| Bytes::from_static(k))
                .to_vec()
        );
    }

    #[tokio::test]
    async fn scan_keys_yields_only_keys() {
        let kv = InMemoryKvStore::new();
        kv.put(Bytes::from_static(b"a"), Bytes::from_static(b"1"))
            .await;
        kv.put(Bytes::from_static(b"b"), Bytes::from_static(b"2"))
            .await;
        let keys: Vec<Bytes> = kv.scan_keys(all()).collect().await;
        assert_eq!(
            keys,
            vec![Bytes::from_static(b"a"), Bytes::from_static(b"b")]
        );
    }

    #[tokio::test]
    async fn prefix_range_scans_exactly_the_prefix() {
        let kv = InMemoryKvStore::new();
        for k in ["seen/a", "seen/b", "seen0", "seem", "tick/x"] {
            kv.put(Bytes::from(k.as_bytes().to_vec()), Bytes::from_static(b"1"))
                .await;
        }
        // "seen/" must match seen/a and seen/b only — NOT "seen0" (next byte after '/') or "seem".
        let keys: Vec<Bytes> = kv.scan_keys(prefix_range(b"seen/")).collect().await;
        assert_eq!(
            keys,
            vec![Bytes::from_static(b"seen/a"), Bytes::from_static(b"seen/b")]
        );
        // an all-0xFF prefix runs to the end (unbounded upper); an empty prefix scans everything.
        assert_eq!(kv.scan_keys(prefix_range(b"")).count().await, 5);
        // a prefix ending in 0xFF still bounds correctly.
        kv.put(Bytes::from_static(&[0xFF, 0x01]), Bytes::from_static(b"x"))
            .await;
        kv.put(Bytes::from_static(&[0xFF, 0xFF]), Bytes::from_static(b"y"))
            .await;
        assert_eq!(kv.scan_keys(prefix_range(&[0xFF])).count().await, 2);
    }

    /// The store drives correctly under Cameron's Bach simulator — put/get/delete and both scans run on the
    /// deterministic discrete-event runtime rather than tokio. Because the trait and in-memory impl are
    /// runtime-agnostic (await-only, no tokio primitives; the scans are snapshot streams), Bach drives them
    /// unchanged — the seam for determinism and snapshot testing.
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
                assert_eq!(kv.scan(prefix_range(b"seen/")).count().await, 2);
                assert_eq!(kv.scan_keys(prefix_range(b"seen/")).count().await, 2);
                assert!(kv.delete(b"seen/msg-1").await);
                assert_eq!(kv.scan_keys(prefix_range(b"seen/")).count().await, 1);
            }
            .group("kv-store")
            .primary()
            .spawn();
        });
    }
}
