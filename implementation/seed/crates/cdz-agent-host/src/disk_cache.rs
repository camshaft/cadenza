//! A bounded on-DISK cache tier over any [`BlobStore`] (operator directive: "multi-tier to disk" — the tier
//! between the in-memory [`CachingBlobStore`](crate::CachingBlobStore) and the backing store, e.g. S3). Blob
//! reads are content-addressed, so a cached file is IMMUTABLE and always correct for its key — a hit needs no
//! invalidation, and eviction is safe (a re-fetch from the inner store repopulates it).
//!
//! **Why a bespoke file cache, not a wrapped [`DiskBlobStore`].** `DiskBlobStore` is a durable content-store
//! with NO delete / enumerate API, so it can't be bounded (no eviction) — and the host must not edit the
//! kernel to add one. This tier manages its OWN cache directory directly: content-hash-named files, a byte
//! budget, and eviction by deleting the oldest cached files. It is a CACHE (evictable, bounded), distinct from
//! `DiskBlobStore`'s durable store.
//!
//! **Bounded by TOTAL BYTES, FIFO eviction.** On insert, if the cache would exceed its byte budget, the
//! OLDEST-inserted files are deleted until the newcomer fits. FIFO (insertion order) not LRU: a disk tier's
//! value is absorbing reuse across a session, and FIFO needs no per-hit disk write (an LRU touch would rewrite
//! metadata on every read) — the in-memory [`CachingBlobStore`] S3-FIFO tier above already does the
//! recency-aware admission; this tier is the larger overflow catch. A blob LARGER than the whole budget is
//! served through without caching (never evict everything for one file). `budget == 0` disables the tier
//! (pure pass-through to the inner store).
//!
//! **Self-verifying + crash-safe writes** (same discipline as `DiskBlobStore`): a cached file is written to a
//! temp path then atomically renamed into place, and a read RE-HASHES the file, refusing (and removing) a file
//! that doesn't match its name — a truncated/corrupt cache file is a miss that re-fetches, never bad bytes.
//!
//! **Boot-enumeration.** On construction the cache dir is scanned to rebuild the in-memory index (which hashes
//! are present + their sizes + a stable insertion order), so a restart reuses the on-disk cache instead of
//! starting cold.
//!
//! **Single-threaded convention.** `?Send` like the other host pieces; [`BlobStore::get`] takes `&self`, so the
//! index (map + FIFO order + byte total) lives behind a [`RefCell`].

use cdz_kernel::blob::BlobStore;
use cdz_kernel::hash::Hash;
use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

/// The in-memory index over the cache dir: byte size per cached hash + FIFO insertion order + the running
/// total. A hash in `sizes` has a file on disk and appears once in `order`.
struct Index {
    sizes: HashMap<Hash, u64>,
    /// Insertion order (front = oldest = next eviction victim). Each cached hash appears exactly once.
    order: VecDeque<Hash>,
    total_bytes: u64,
}

/// A bounded on-disk cache tier in front of an inner [`BlobStore`] `B`. Caches blobs as content-hash-named
/// files under `dir`, bounded to `budget` total bytes with FIFO eviction.
pub struct DiskCacheTier<B> {
    inner: B,
    dir: PathBuf,
    /// Max total bytes of cached files. `0` disables the tier (pass-through).
    budget: u64,
    index: RefCell<Index>,
}

impl<B> DiskCacheTier<B> {
    /// Wrap `inner` with an on-disk cache at `dir`, bounded to `max_bytes` total. Creates `dir` if absent and
    /// SCANS it to rebuild the index from any pre-existing cache files (boot-enumeration — a restart reuses
    /// the warm cache). `max_bytes == 0` disables caching (pass-through); the dir is still created (harmless).
    /// A dir-scan I/O error starts with an empty index (the cache is best-effort — it must never fail the
    /// host; a re-fetch repopulates).
    pub fn new(inner: B, dir: impl Into<PathBuf>, max_bytes: u64) -> Self {
        let dir = dir.into();
        // A DISABLED tier (budget 0) touches NO filesystem — don't create/scan the dir (so the daemon can
        // wrap unconditionally with an empty/unused dir when the disk tier is off, exactly like the mem
        // CachingBlobStore's budget-0 pass-through). Only a real budget creates + boot-scans the cache dir.
        if max_bytes == 0 {
            return DiskCacheTier {
                inner,
                dir,
                budget: 0,
                index: RefCell::new(Index {
                    sizes: HashMap::new(),
                    order: VecDeque::new(),
                    total_bytes: 0,
                }),
            };
        }
        let _ = std::fs::create_dir_all(&dir);
        let index = Self::scan_dir(&dir);
        DiskCacheTier {
            inner,
            dir,
            budget: max_bytes,
            index: RefCell::new(index),
        }
    }

    /// Rebuild the index by listing `dir`: each entry whose filename is a valid hash-hex is a cached blob;
    /// record its size. Order across a scan is filesystem-arbitrary (a cold-start detail — FIFO order only has
    /// to be SOME consistent order; fresh inserts append in real order). A non-hex / unreadable entry is
    /// skipped (foreign files don't corrupt the index).
    fn scan_dir(dir: &Path) -> Index {
        let mut sizes = HashMap::new();
        let mut order = VecDeque::new();
        let mut total_bytes = 0u64;
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let Some(name) = name.to_str() else { continue };
                let Some(hash) = Hash::from_hex(name) else {
                    continue;
                };
                let Ok(meta) = entry.metadata() else { continue };
                if !meta.is_file() {
                    continue;
                }
                let len = meta.len();
                sizes.insert(hash, len);
                order.push_back(hash);
                total_bytes += len;
            }
        }
        Index {
            sizes,
            order,
            total_bytes,
        }
    }

    /// The cache file path for `hash` — a flat `<dir>/<hex>` (the cache is a single dir; unlike `DiskBlobStore`
    /// it isn't sharded since it's bounded/evicted, not a large durable keyspace).
    fn path_for(&self, hash: &Hash) -> PathBuf {
        self.dir.join(hash.to_hex())
    }

    /// Current cached file count — for tests/inspection.
    pub fn cached_entries(&self) -> usize {
        self.index.borrow().sizes.len()
    }

    /// Current total cached bytes — the quantity the budget bounds.
    pub fn cached_bytes(&self) -> u64 {
        self.index.borrow().total_bytes
    }

    /// Write `bytes` under `hash` into the cache dir (atomic temp+rename), evicting oldest files first to stay
    /// within budget. A blob larger than the whole budget is NOT cached (served through). Already-cached =
    /// no-op (content-addressed → identical bytes). Best-effort: an I/O failure just leaves it uncached.
    fn cache_write(&self, hash: &Hash, bytes: &[u8]) {
        let len = bytes.len() as u64;
        if self.budget == 0 || len > self.budget {
            return;
        }
        {
            if self.index.borrow().sizes.contains_key(hash) {
                return;
            }
        }
        // Evict oldest until the newcomer fits.
        self.evict_to_fit(len);
        // Atomic write: temp file in the same dir, then rename (intra-dir rename is atomic).
        let final_path = self.path_for(hash);
        let tmp_path = self.dir.join(format!("{}.tmp", hash.to_hex()));
        if std::fs::write(&tmp_path, bytes).is_err() {
            let _ = std::fs::remove_file(&tmp_path);
            return;
        }
        if std::fs::rename(&tmp_path, &final_path).is_err() {
            let _ = std::fs::remove_file(&tmp_path);
            return;
        }
        let mut idx = self.index.borrow_mut();
        idx.sizes.insert(*hash, len);
        idx.order.push_back(*hash);
        idx.total_bytes += len;
    }

    /// Delete oldest-inserted cache files until `total_bytes + incoming <= budget` (or the cache is empty).
    fn evict_to_fit(&self, incoming: u64) {
        let mut idx = self.index.borrow_mut();
        while idx.total_bytes + incoming > self.budget {
            let Some(victim) = idx.order.pop_front() else {
                break;
            };
            if let Some(len) = idx.sizes.remove(&victim) {
                idx.total_bytes -= len;
                let _ = std::fs::remove_file(self.dir.join(victim.to_hex()));
            }
        }
    }

    /// Read a cached blob from disk, VERIFYING it hashes to `hash`. A mismatch (corrupt/truncated file) or a
    /// read error is treated as a MISS (returns None) AND the bad file is removed + de-indexed, so a re-fetch
    /// repopulates it cleanly. Only called when the index says the hash is present.
    fn cache_read(&self, hash: &Hash) -> Option<Vec<u8>> {
        let path = self.path_for(hash);
        match std::fs::read(&path) {
            Ok(bytes) if Hash::of(&bytes) == *hash => Some(bytes),
            _ => {
                // Corrupt/absent/mismatch: drop it from the cache so a re-fetch rewrites it.
                let mut idx = self.index.borrow_mut();
                if let Some(len) = idx.sizes.remove(hash) {
                    idx.total_bytes -= len;
                    if let Some(pos) = idx.order.iter().position(|h| h == hash) {
                        idx.order.remove(pos);
                    }
                }
                let _ = std::fs::remove_file(&path);
                None
            }
        }
    }
}

#[async_trait::async_trait(?Send)]
impl<B: BlobStore> BlobStore for DiskCacheTier<B> {
    /// Write THROUGH to the inner store under the SUPPLIED hash (computed once by the caller — no re-hash per
    /// tier), then populate the disk cache. `Bytes` moves into the inner `put`; the cache write borrows it.
    async fn put(&mut self, hash: Hash, bytes: bytes::Bytes) -> std::io::Result<()> {
        self.cache_write(&hash, &bytes);
        self.inner.put(hash, bytes).await
    }

    /// Serve from the disk cache on a hit (verified); on a miss, fetch from the inner store and populate the
    /// cache (promote-on-hit). Content-addressed → a cached file is always valid for its key, no invalidation.
    async fn get(&self, hash: &Hash) -> std::io::Result<Option<bytes::Bytes>> {
        if self.budget > 0 && self.index.borrow().sizes.contains_key(hash) {
            if let Some(bytes) = self.cache_read(hash) {
                return Ok(Some(bytes.into()));
            }
            // cache_read found a corrupt file (already de-indexed + removed) — fall through to the inner store.
        }
        let fetched = self.inner.get(hash).await?;
        if let Some(bytes) = &fetched {
            self.cache_write(hash, bytes);
        }
        Ok(fetched)
    }

    /// Existence probe: a cached file trivially exists; otherwise defer to the inner store's probe (cheaper
    /// than a fetch on a real backend). Doesn't populate (a `has` fetches no bytes).
    async fn has(&self, hash: &Hash) -> std::io::Result<bool> {
        if self.budget > 0 && self.index.borrow().sizes.contains_key(hash) {
            return Ok(true);
        }
        self.inner.has(hash).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_kernel::blob::MemBlobStore;

    fn blob(b: u8, n: usize) -> bytes::Bytes {
        bytes::Bytes::from(vec![b; n])
    }

    /// Test helper mirroring the OLD `put(&bytes) -> Hash` ergonomics over the new `put(hash, Bytes) -> ()`:
    /// compute the content hash once, store, return the hash (what the tests key on).
    async fn put_blob<S: BlobStore>(store: &mut S, bytes: &bytes::Bytes) -> Hash {
        let hash = Hash::of(bytes);
        store.put(hash, bytes.clone()).await.unwrap();
        hash
    }

    /// A unique proven-fresh cache dir per test (no fixed temp path).
    fn cache_dir(tag: &str) -> PathBuf {
        crate::testutil::unique_temp_dir(tag)
    }

    /// An inner store that records get() calls + can go blind, to prove a disk-cache hit skips the inner store.
    struct CountingBlobStore {
        inner: MemBlobStore,
        gets: std::cell::Cell<usize>,
        blind: std::cell::Cell<bool>,
    }
    #[async_trait::async_trait(?Send)]
    impl BlobStore for CountingBlobStore {
        async fn put(&mut self, hash: Hash, bytes: bytes::Bytes) -> std::io::Result<()> {
            self.inner.put(hash, bytes).await
        }
        async fn get(&self, hash: &Hash) -> std::io::Result<Option<bytes::Bytes>> {
            self.gets.set(self.gets.get() + 1);
            if self.blind.get() {
                return Ok(None);
            }
            self.inner.get(hash).await
        }
    }

    #[tokio::test]
    async fn hit_after_miss_serves_from_disk_without_rehitting_inner() {
        let dir = cache_dir("diskcache-hit");
        let mut inner = MemBlobStore::new();
        let bytes = blob(1, 100);
        let hash = put_blob(&mut inner, &bytes).await;
        let counting = CountingBlobStore {
            inner,
            gets: std::cell::Cell::new(0),
            blind: std::cell::Cell::new(false),
        };
        let c = DiskCacheTier::new(counting, &dir, 1024);
        assert_eq!(c.get(&hash).await.unwrap().as_deref(), Some(&bytes[..]));
        assert_eq!(
            c.inner.gets.get(),
            1,
            "the miss consulted the inner store once"
        );
        c.inner.blind.set(true);
        assert_eq!(
            c.get(&hash).await.unwrap().as_deref(),
            Some(&bytes[..]),
            "served from the disk cache despite a blinded inner store"
        );
        assert_eq!(
            c.inner.gets.get(),
            1,
            "the disk hit did NOT re-hit the inner store"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn fifo_evicts_oldest_at_the_byte_bound() {
        // Budget 250. Put A(100), B(100) [200], then C(100): total would be 300 > 250 → evict A (oldest).
        let dir = cache_dir("diskcache-evict");
        let mut c = DiskCacheTier::new(MemBlobStore::new(), &dir, 250);
        let a = blob(b'a', 100);
        let bb = blob(b'b', 100);
        let cc = blob(b'c', 100);
        let ha = put_blob(&mut c, &a).await;
        let _hb = put_blob(&mut c, &bb).await;
        assert_eq!(c.cached_bytes(), 200);
        let _hc = put_blob(&mut c, &cc).await;
        assert_eq!(c.cached_bytes(), 200, "stayed within the 250 budget");
        assert!(
            !dir.join(ha.to_hex()).exists(),
            "A (oldest) was evicted — its file is gone"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_blob_larger_than_the_budget_is_served_but_not_cached() {
        let dir = cache_dir("diskcache-oversized");
        let mut c = DiskCacheTier::new(MemBlobStore::new(), &dir, 50);
        let big = blob(9, 100);
        let h = put_blob(&mut c, &big).await;
        assert_eq!(c.cached_bytes(), 0, "oversized blob not cached");
        assert_eq!(
            c.get(&h).await.unwrap().as_deref(),
            Some(&big[..]),
            "still served from the inner store"
        );
        assert_eq!(c.cached_bytes(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn budget_zero_is_a_pure_passthrough() {
        let dir = cache_dir("diskcache-zero");
        let mut c = DiskCacheTier::new(MemBlobStore::new(), &dir, 0);
        let bytes = blob(7, 100);
        let h = put_blob(&mut c, &bytes).await;
        assert_eq!(c.cached_bytes(), 0);
        assert_eq!(c.get(&h).await.unwrap().as_deref(), Some(&bytes[..]));
        assert_eq!(c.cached_bytes(), 0, "disabled tier never caches");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn boot_enumeration_reuses_a_warm_on_disk_cache() {
        // Populate a cache, drop it, then construct a NEW tier over the SAME dir + a blinded inner store: the
        // boot scan rebuilds the index, so the blob is served from the warm disk cache without the inner.
        let dir = cache_dir("diskcache-warm");
        let bytes = blob(5, 120);
        let hash = {
            let mut c = DiskCacheTier::new(MemBlobStore::new(), &dir, 1024);
            let h = put_blob(&mut c, &bytes).await;
            assert_eq!(c.cached_bytes(), 120);
            h
        };
        // Fresh tier, same dir, inner store that has NOTHING (proves the disk cache served it).
        let c2 = DiskCacheTier::new(MemBlobStore::new(), &dir, 1024);
        assert_eq!(
            c2.cached_bytes(),
            120,
            "boot scan rebuilt the index from the warm dir"
        );
        assert_eq!(
            c2.get(&hash).await.unwrap().as_deref(),
            Some(&bytes[..]),
            "the warm on-disk cache served the blob after a restart"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_corrupt_cache_file_is_a_miss_that_refetches() {
        // Tamper a cache file so it no longer hashes to its name: the read detects the mismatch, removes it,
        // and falls through to the inner store (which still has the real bytes).
        let dir = cache_dir("diskcache-corrupt");
        let mut inner = MemBlobStore::new();
        let bytes = blob(4, 100);
        let hash = put_blob(&mut inner, &bytes).await;
        let c = DiskCacheTier::new(inner, &dir, 1024);
        // Populate the cache via a get (miss → inner → cache).
        assert_eq!(c.get(&hash).await.unwrap().as_deref(), Some(&bytes[..]));
        assert_eq!(c.cached_bytes(), 100);
        // Corrupt the on-disk file.
        std::fs::write(dir.join(hash.to_hex()), b"tampered bytes").unwrap();
        // A get re-hashes, detects the mismatch, removes it, and re-fetches from the inner store.
        assert_eq!(
            c.get(&hash).await.unwrap().as_deref(),
            Some(&bytes[..]),
            "a corrupt cache file is a miss that re-fetches the real bytes"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
