//! A bounded in-memory [`BlobStore`] cache with **S3-FIFO** eviction (`MemoryCache`) — the shallowest tier
//! of a [`TieredBlobStore`](crate::TieredBlobStore).
//!
//! S3-FIFO (Yang et al., 2023) is a simple, scan-resistant policy that beats LRU on hit-rate without LRU's
//! per-access list surgery. Three FIFO queues:
//! - **small (`S`)** — new insertions land here (a probation queue for one-hit-wonders); target ≈ 10% of
//!   the byte budget.
//! - **main (`M`)** — items that proved useful get promoted here; ≈ 90% of the budget.
//! - **ghost (`G`)** — keys recently evicted from `S` (metadata only, no bytes); a key seen again while in
//!   `G` is "hot" and inserted straight into `M`.
//!
//! Each live entry carries a small **frequency** counter (0–3), bumped on a `get` hit. Eviction: if `S` is
//! over its target, evict from `S` — a `freq > 0` item is promoted to `M` (reset), else evicted (and its
//! key recorded in `G`); otherwise evict from `M` with a second chance (a `freq > 0` item is reinserted
//! with `freq -= 1`, else evicted). The cache is **byte-budgeted** (blobs vary in size); a blob larger than
//! the whole budget is simply not cached (the durable tier still holds it).
//!
//! It is a CACHE, so it never has to hold anything — every op is infallible (`Ok`), and a `get` miss just
//! falls through to a deeper tier.

use async_trait::async_trait;
use bytes::Bytes;
use cdz_platform::{BlobStore, BlobStoreError, Hash, HashTag};
use std::collections::HashMap;
use std::collections::{HashSet, VecDeque};
use std::sync::Mutex;

/// The 32-byte content digest a blob is keyed on (tag-agnostic, matching `InMemoryBlobStore`).
type Digest = [u8; Hash::DIGEST_LEN];

struct Entry {
    bytes: Bytes,
    /// Access frequency, saturating at [`MAX_FREQ`]; bumped on a `get` hit, consumed by eviction.
    freq: u8,
}

/// The saturating cap on an entry's frequency counter (S3-FIFO uses a small cap, typically 3).
const MAX_FREQ: u8 = 3;

struct Inner {
    /// Live entries (present in `small` or `main`), keyed by digest.
    map: HashMap<Digest, Entry>,
    small: VecDeque<Digest>,
    main: VecDeque<Digest>,
    /// Keys recently evicted from `small` (bytes already dropped) — bounds re-admission.
    ghost: VecDeque<Digest>,
    ghost_set: HashSet<Digest>,
    small_bytes: usize,
    main_bytes: usize,
    /// Total byte budget across `small` + `main`.
    capacity: usize,
    /// Target size of `small` in bytes (≈ 10% of `capacity`).
    small_cap: usize,
    /// Max number of keys retained in `ghost`.
    ghost_cap: usize,
}

/// A bounded in-memory `BlobStore` cache using S3-FIFO eviction. Cheap to share behind an `Arc` (all state
/// is behind one `Mutex`; critical sections are map/queue ops with no `await` held).
pub struct MemoryCache {
    inner: Mutex<Inner>,
}

impl MemoryCache {
    /// A cache holding at most `capacity_bytes` of blob bytes (across the small + main queues), evicting by
    /// S3-FIFO once full.
    #[must_use]
    pub fn with_capacity(capacity_bytes: usize) -> Self {
        let small_cap = (capacity_bytes / 10).max(1);
        // Bound the ghost queue by a key count. Keys are tiny (32 bytes); a loose bound proportional to the
        // budget keeps re-admission useful without unbounded growth.
        let ghost_cap = (capacity_bytes / 4096).clamp(16, 1 << 20);
        Self {
            inner: Mutex::new(Inner {
                map: HashMap::new(),
                small: VecDeque::new(),
                main: VecDeque::new(),
                ghost: VecDeque::new(),
                ghost_set: HashSet::new(),
                small_bytes: 0,
                main_bytes: 0,
                capacity: capacity_bytes,
                small_cap,
                ghost_cap,
            }),
        }
    }

    /// The total bytes currently cached (small + main). For tests/introspection.
    #[must_use]
    pub fn current_bytes(&self) -> usize {
        let inner = self.inner.lock().expect("mem cache lock not poisoned");
        inner.small_bytes + inner.main_bytes
    }

    /// The number of blobs currently cached. For tests/introspection.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .expect("mem cache lock not poisoned")
            .map
            .len()
    }

    /// Whether the cache holds no blobs.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Inner {
    /// Record `d` as recently-evicted-from-small, bounding the ghost queue.
    fn push_ghost(&mut self, d: Digest) {
        if self.ghost_set.insert(d) {
            self.ghost.push_back(d);
        }
        while self.ghost_set.len() > self.ghost_cap {
            match self.ghost.pop_front() {
                Some(old) => {
                    self.ghost_set.remove(&old);
                }
                None => break,
            }
        }
    }

    /// Evict until the total cached bytes fit the budget.
    fn evict_to_fit(&mut self) {
        while self.small_bytes + self.main_bytes > self.capacity {
            let evict_small = self.small_bytes >= self.small_cap && !self.small.is_empty();
            if evict_small {
                self.evict_from_small();
            } else if !self.main.is_empty() {
                self.evict_from_main();
            } else if !self.small.is_empty() {
                self.evict_from_small();
            } else {
                break; // nothing left to evict
            }
        }
    }

    /// One eviction step from `small`: promote a warm item to `main` (reset freq) or evict a cold one (into
    /// ghost). Returns after a single structural action so the caller re-checks the budget.
    fn evict_from_small(&mut self) {
        while let Some(d) = self.small.pop_front() {
            let Some(entry) = self.map.get_mut(&d) else {
                continue; // stale queue entry, skip
            };
            let size = entry.bytes.len();
            if entry.freq > 0 {
                entry.freq = 0;
                self.small_bytes -= size;
                self.main.push_back(d);
                self.main_bytes += size;
            } else {
                self.map.remove(&d);
                self.small_bytes -= size;
                self.push_ghost(d);
            }
            return;
        }
    }

    /// One eviction from `main`: give warm items a second chance (`freq -= 1`, reinsert) until a cold one
    /// (`freq == 0`) is found and evicted.
    fn evict_from_main(&mut self) {
        while let Some(d) = self.main.pop_front() {
            let Some(entry) = self.map.get_mut(&d) else {
                continue; // stale
            };
            if entry.freq > 0 {
                entry.freq -= 1;
                self.main.push_back(d);
                continue;
            }
            let size = entry.bytes.len();
            self.map.remove(&d);
            self.main_bytes -= size;
            return;
        }
    }
}

#[async_trait]
impl BlobStore for MemoryCache {
    async fn put(&self, bytes: Bytes) -> Result<Hash, BlobStoreError> {
        let hash = Hash::of(HashTag::Blob, &bytes);
        let digest = *hash.digest();
        let size = bytes.len();
        let mut inner = self.inner.lock().expect("mem cache lock not poisoned");

        // Already cached (content-addressed → identical bytes): just warm it.
        if let Some(entry) = inner.map.get_mut(&digest) {
            entry.freq = (entry.freq + 1).min(MAX_FREQ);
            return Ok(hash);
        }
        // A blob larger than the whole budget can't be usefully cached — skip it (the durable tier holds it).
        if size > inner.capacity {
            return Ok(hash);
        }

        inner.map.insert(digest, Entry { bytes, freq: 0 });
        // A key seen again while in the ghost queue is "hot" → straight to main; else probation in small.
        if inner.ghost_set.remove(&digest) {
            inner.main.push_back(digest);
            inner.main_bytes += size;
        } else {
            inner.small.push_back(digest);
            inner.small_bytes += size;
        }
        inner.evict_to_fit();
        Ok(hash)
    }

    async fn get(&self, hash: Hash) -> Result<Option<Bytes>, BlobStoreError> {
        let mut inner = self.inner.lock().expect("mem cache lock not poisoned");
        match inner.map.get_mut(hash.digest()) {
            Some(entry) => {
                entry.freq = (entry.freq + 1).min(MAX_FREQ);
                Ok(Some(entry.bytes.clone()))
            }
            None => Ok(None),
        }
    }

    async fn has(&self, hash: Hash) -> Result<bool, BlobStoreError> {
        let inner = self.inner.lock().expect("mem cache lock not poisoned");
        Ok(inner.map.contains_key(hash.digest()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob(n: usize) -> Bytes {
        // 100-byte blobs with genuinely distinct content per `n` (n encoded in the leading bytes), so no two
        // distinct `n` collide to the same hash.
        let mut v = n.to_le_bytes().to_vec();
        v.resize(100, 0);
        Bytes::from(v)
    }
    fn blob_hash(bytes: &Bytes) -> Hash {
        Hash::of(HashTag::Blob, bytes)
    }

    #[tokio::test]
    async fn put_get_round_trips_and_reports_absence() {
        let cache = MemoryCache::with_capacity(10_000);
        let bytes = Bytes::from_static(b"cache me");
        let hash = cache.put(bytes.clone()).await.unwrap();
        assert_eq!(cache.get(hash).await.unwrap(), Some(bytes));
        assert!(cache.has(hash).await.unwrap());
        let absent = Hash::of(HashTag::Blob, b"never");
        assert_eq!(cache.get(absent).await.unwrap(), None);
        assert!(!cache.has(absent).await.unwrap());
    }

    #[tokio::test]
    async fn respects_the_byte_budget_under_pressure() {
        // 1 KiB budget, 100-byte blobs → at most ~10 fit; inserting 100 must not exceed the budget.
        let cache = MemoryCache::with_capacity(1024);
        for i in 0..100 {
            cache.put(blob(i * 997 + 1)).await.unwrap(); // distinct-ish content
        }
        assert!(
            cache.current_bytes() <= 1024,
            "cached bytes {} must be within the 1024 budget",
            cache.current_bytes()
        );
    }

    #[tokio::test]
    async fn evicts_cold_items_keeps_recent() {
        let cache = MemoryCache::with_capacity(500); // ~5 blobs
        let first = blob(1);
        let fh = cache.put(first).await.unwrap();
        // Flood with many distinct cold blobs — the untouched first one gets evicted.
        for i in 2..50 {
            cache.put(blob(i)).await.unwrap();
        }
        assert_eq!(
            cache.get(fh).await.unwrap(),
            None,
            "a cold early blob is evicted under pressure"
        );
    }

    #[tokio::test]
    async fn a_hot_item_survives_a_flood() {
        let cache = MemoryCache::with_capacity(500);
        let hot = blob(1);
        let hh = cache.put(hot.clone()).await.unwrap();
        // Touch it repeatedly so it earns frequency (and gets promoted to main).
        for _ in 0..5 {
            let _ = cache.get(hh).await.unwrap();
            // Interleave cold inserts.
            for i in 2..12 {
                cache.put(blob(i * 131 + 7)).await.unwrap();
            }
            let _ = cache.get(hh).await.unwrap();
        }
        assert_eq!(
            cache.get(hh).await.unwrap(),
            Some(hot),
            "a frequently-read blob survives eviction pressure"
        );
    }

    #[tokio::test]
    async fn a_blob_larger_than_the_budget_is_not_cached() {
        let cache = MemoryCache::with_capacity(64);
        let big = Bytes::from(vec![0u8; 1000]);
        let hash = cache.put(big).await.unwrap();
        // put "succeeds" (returns the hash) but the oversized blob isn't retained.
        assert_eq!(cache.get(hash).await.unwrap(), None);
        assert_eq!(cache.current_bytes(), 0);
    }

    #[tokio::test]
    async fn put_is_idempotent_by_content() {
        let cache = MemoryCache::with_capacity(10_000);
        let bytes = Bytes::from_static(b"same bytes");
        let h1 = cache.put(bytes.clone()).await.unwrap();
        let h2 = cache.put(bytes.clone()).await.unwrap();
        assert_eq!(h1, h2);
        assert_eq!(cache.len(), 1);
        assert_eq!(h1, blob_hash(&bytes));
    }
}
