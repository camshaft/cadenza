//! A bounded-byte-budget LRU cache DECORATOR over any [`BlobStore`] (operator directive: "for the blob store
//! we should have a bounded LRU cache"). Blob reads are content-addressed, so this is the cleanest possible
//! cache: a [`Hash`] maps to fixed, immutable bytes — a hit is ALWAYS correct and there is NO invalidation
//! (the value for a key can never change). Repeated fetches of the same blob (a reducer component re-read per
//! install, a doc queried back) then serve from memory instead of re-hitting S3 / disk each time.
//!
//! **Pure host MECHANISM, no policy** (minimize-host-logic norm). It wraps ANY `BlobStore` — `MemBlobStore`,
//! `DiskBlobStore`, or the AWS `S3BlobStore` — behind the SAME trait, so a consumer holds a
//! `CachingBlobStore<B>` exactly where it held `B`; the decorator composes like the metered-executor wrapper.
//!
//! **Bounded by TOTAL BYTES, not entry count.** Blobs vary widely in size (a small doc vs a multi-MiB reducer
//! component), so a byte budget is the right bound for a blob cache — an entry-count bound would let a few
//! large blobs blow memory, or cap tiny blobs pointlessly. Eviction is LRU: on insert, least-recently-used
//! entries are dropped until the newcomer fits. A single blob LARGER than the whole budget is never cached
//! (served straight through) rather than evicting everything to hold one oversized value.
//!
//! **Single-threaded convention.** The kernel traits are `?Send` (single-threaded by design), and
//! [`BlobStore::get`] takes `&self`, so the cache state (map + recency order) lives behind a [`RefCell`] —
//! `get` mutates recency through a shared reference with no locking, matching the crate's single-task host loop.

use cdz_kernel::blob::BlobStore;
use cdz_kernel::hash::Hash;
use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::VecDeque;

/// The cache state behind a [`RefCell`]: the stored bytes per hash + an LRU recency queue (front =
/// least-recently-used, back = most-recently-used) + the running total of cached bytes. Kept together so a
/// hit/insert/evict updates all three atomically under one borrow.
struct CacheState {
    /// The cached blobs. A key is present iff its bytes are counted in `total_bytes` and it appears once in
    /// `recency`.
    entries: HashMap<Hash, Vec<u8>>,
    /// LRU order: `recency.front()` is the least-recently-used key (the next eviction victim),
    /// `recency.back()` the most-recently-used. Every key in `entries` appears EXACTLY once here.
    recency: VecDeque<Hash>,
    /// Sum of `entries` value lengths — the bound is on this, not on `entries.len()`.
    total_bytes: usize,
}

impl CacheState {
    /// Move `hash` to the most-recently-used end (called on a hit and after an insert). O(n) in the queue
    /// length to find+remove the old position; the cache is bounded so n is small, and this keeps the
    /// dependency-free (no external LRU crate) — the whole cache is one map + one deque.
    fn touch(&mut self, hash: &Hash) {
        if let Some(pos) = self.recency.iter().position(|h| h == hash) {
            self.recency.remove(pos);
        }
        self.recency.push_back(*hash);
    }

    /// Drop the least-recently-used entries until `total_bytes + incoming` fits within `budget` (or the cache
    /// is empty). Called before an insert so the newcomer fits. An entry present in `recency` is always
    /// present in `entries` (invariant), so the removes are consistent.
    fn evict_to_fit(&mut self, incoming: usize, budget: usize) {
        while self.total_bytes + incoming > budget {
            let Some(victim) = self.recency.pop_front() else {
                // Nothing left to evict — the incoming blob is larger than the whole budget on an empty
                // cache. The caller handles the "don't cache an oversized blob" case; break defensively.
                break;
            };
            if let Some(bytes) = self.entries.remove(&victim) {
                self.total_bytes -= bytes.len();
            }
        }
    }
}

/// A bounded-byte-budget LRU cache in front of an inner [`BlobStore`] `B`. Construct with
/// [`new`](Self::new) (a byte budget) and use it anywhere a `BlobStore` is expected.
pub struct CachingBlobStore<B> {
    inner: B,
    /// The maximum total bytes the cache may hold. `0` disables caching entirely (every `get`/`put` is a
    /// straight pass-through with zero bookkeeping) — the operator's "off" setting.
    budget: usize,
    state: RefCell<CacheState>,
}

impl<B> CachingBlobStore<B> {
    /// Wrap `inner` with a cache bounded to `max_bytes` total. `max_bytes == 0` disables the cache (pure
    /// pass-through) — a caller that doesn't want caching pays nothing.
    pub fn new(inner: B, max_bytes: usize) -> Self {
        CachingBlobStore {
            inner,
            budget: max_bytes,
            state: RefCell::new(CacheState {
                entries: HashMap::new(),
                recency: VecDeque::new(),
                total_bytes: 0,
            }),
        }
    }

    /// Insert `bytes` under `hash`, evicting LRU entries to stay within budget. A blob larger than the whole
    /// budget is NOT cached (it would force evicting everything to hold one value) — it's simply served
    /// through without being retained. A blob already cached is moved to most-recently-used (no double-count).
    fn cache_put(&self, hash: Hash, bytes: &[u8]) {
        if self.budget == 0 || bytes.len() > self.budget {
            return;
        }
        let mut st = self.state.borrow_mut();
        if st.entries.contains_key(&hash) {
            // Already cached (content-addressed → identical bytes); just refresh recency.
            st.touch(&hash);
            return;
        }
        st.evict_to_fit(bytes.len(), self.budget);
        st.total_bytes += bytes.len();
        st.entries.insert(hash, bytes.to_vec());
        st.touch(&hash);
    }

    /// The current number of cached blobs — for tests/inspection (the bound is on bytes, not this count).
    pub fn cached_entries(&self) -> usize {
        self.state.borrow().entries.len()
    }

    /// The current total cached bytes — the quantity the budget bounds.
    pub fn cached_bytes(&self) -> usize {
        self.state.borrow().total_bytes
    }
}

#[async_trait::async_trait(?Send)]
impl<B: BlobStore> BlobStore for CachingBlobStore<B> {
    /// Write THROUGH to the inner store, then populate the cache with the just-stored bytes (a fresh `put` is
    /// very often followed by a `get` of the same hash — e.g. store a doc then immediately query it back).
    async fn put(&mut self, bytes: &[u8]) -> std::io::Result<Hash> {
        let hash = self.inner.put(bytes).await?;
        self.cache_put(hash, bytes);
        Ok(hash)
    }

    /// Serve from cache on a hit (bumping recency); on a miss, fetch from the inner store and populate the
    /// cache. Content-addressed keys mean a cached value is always valid for its key — no invalidation.
    async fn get(&self, hash: &Hash) -> std::io::Result<Option<Vec<u8>>> {
        if self.budget > 0 {
            let mut st = self.state.borrow_mut();
            if let Some(bytes) = st.entries.get(hash).cloned() {
                st.touch(hash);
                return Ok(Some(bytes));
            }
        }
        // Miss: fetch from the backing store, then cache it (borrow of `state` is already released above).
        let fetched = self.inner.get(hash).await?;
        if let Some(bytes) = &fetched {
            self.cache_put(*hash, bytes);
        }
        Ok(fetched)
    }

    /// Existence probe: a cached blob trivially exists; otherwise defer to the inner store's probe (which a
    /// real backend implements more cheaply than a full fetch). Doesn't populate the cache (a `has` doesn't
    /// fetch bytes).
    async fn has(&self, hash: &Hash) -> std::io::Result<bool> {
        if self.budget > 0 && self.state.borrow().entries.contains_key(hash) {
            return Ok(true);
        }
        self.inner.has(hash).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_kernel::blob::MemBlobStore;

    /// A blob of `n` bytes, all `b`. Its hash is content-derived, so distinct (b, n) give distinct keys.
    fn blob(b: u8, n: usize) -> Vec<u8> {
        vec![b; n]
    }

    /// A BlobStore that records every get() call and can be told to "go blind" (return None regardless), so a
    /// test can prove a cache HIT never consulted the inner store.
    struct CountingBlobStore {
        inner: MemBlobStore,
        gets: std::cell::Cell<usize>,
        blind: std::cell::Cell<bool>,
    }
    #[async_trait::async_trait(?Send)]
    impl BlobStore for CountingBlobStore {
        async fn put(&mut self, bytes: &[u8]) -> std::io::Result<Hash> {
            self.inner.put(bytes).await
        }
        async fn get(&self, hash: &Hash) -> std::io::Result<Option<Vec<u8>>> {
            self.gets.set(self.gets.get() + 1);
            if self.blind.get() {
                return Ok(None);
            }
            self.inner.get(hash).await
        }
    }

    #[tokio::test]
    async fn hit_after_miss_serves_from_cache_without_rehitting_inner() {
        // Get a blob once (miss → fetches inner, populates cache), then blind the inner store and get again:
        // the second get must still return the bytes (from cache) AND must NOT have called inner.get again.
        let mut inner = MemBlobStore::new();
        let bytes = blob(1, 100);
        let hash = inner.put(&bytes).await.unwrap();
        let counting = CountingBlobStore {
            inner,
            gets: std::cell::Cell::new(0),
            blind: std::cell::Cell::new(false),
        };
        let c = CachingBlobStore::new(counting, 1024);
        // First get: a miss that consults the inner store once and populates the cache.
        assert_eq!(c.get(&hash).await.unwrap().as_deref(), Some(&bytes[..]));
        assert_eq!(
            c.inner.gets.get(),
            1,
            "the miss consulted the inner store once"
        );
        // Blind the inner store: any further inner.get returns None. A cache HIT must not consult it.
        c.inner.blind.set(true);
        assert_eq!(
            c.get(&hash).await.unwrap().as_deref(),
            Some(&bytes[..]),
            "served from cache despite a blinded inner store"
        );
        assert_eq!(
            c.inner.gets.get(),
            1,
            "the hit did NOT re-hit the inner store"
        );
        assert_eq!(c.cached_entries(), 1);
    }

    #[tokio::test]
    async fn miss_populates_the_cache_from_the_inner_store() {
        // Pre-load the inner store, wrap it, then get a hash the cache hasn't seen: the miss fetches from
        // inner and populates the cache, so cached_bytes reflects it afterward.
        let mut inner = MemBlobStore::new();
        let bytes = blob(2, 200);
        let hash = inner.put(&bytes).await.unwrap();
        let c = CachingBlobStore::new(inner, 1024);
        assert_eq!(c.cached_bytes(), 0, "nothing cached before the first get");
        let got = c.get(&hash).await.unwrap();
        assert_eq!(got.as_deref(), Some(&bytes[..]));
        assert_eq!(c.cached_bytes(), 200, "the miss populated the cache");
        assert_eq!(c.cached_entries(), 1);
    }

    #[tokio::test]
    async fn lru_evicts_least_recently_used_at_the_byte_bound() {
        // Budget = 250 bytes. Insert A(100) then B(100) [total 200], get A (A now most-recently-used), then
        // insert C(100): total would be 300 > 250, so the LRU victim B is evicted, not A.
        let mut c = CachingBlobStore::new(MemBlobStore::new(), 250);
        let a = blob(b'a', 100);
        let bb = blob(b'b', 100);
        let cc = blob(b'c', 100);
        let ha = c.put(&a).await.unwrap();
        let hb = c.put(&bb).await.unwrap();
        assert_eq!(c.cached_bytes(), 200);
        // Touch A so B becomes the LRU victim.
        let _ = c.get(&ha).await.unwrap();
        let hc = c.put(&cc).await.unwrap();
        // A and C are cached (200 bytes); B was evicted to fit C within the 250 budget.
        assert_eq!(c.cached_bytes(), 200);
        assert!(
            c.state.borrow().entries.contains_key(&ha),
            "A was touched, kept"
        );
        assert!(
            c.state.borrow().entries.contains_key(&hc),
            "C is the newcomer"
        );
        assert!(
            !c.state.borrow().entries.contains_key(&hb),
            "B was least-recently-used, evicted"
        );
        // B still served from the inner store (write-through kept it there) — eviction is cache-only.
        assert_eq!(c.get(&hb).await.unwrap().as_deref(), Some(&bb[..]));
    }

    #[tokio::test]
    async fn a_blob_larger_than_the_budget_is_served_but_not_cached() {
        // Budget 50, blob 100: it's stored in the inner store (put write-through) and served on get, but
        // never retained in the cache (caching it would force evicting everything to hold one oversized
        // value). The cache stays empty; the get still returns the bytes (via the inner store).
        let mut c = CachingBlobStore::new(MemBlobStore::new(), 50);
        let big = blob(9, 100);
        let h = c.put(&big).await.unwrap();
        assert_eq!(c.cached_bytes(), 0, "oversized blob not cached on put");
        let got = c.get(&h).await.unwrap();
        assert_eq!(
            got.as_deref(),
            Some(&big[..]),
            "still served from the inner store"
        );
        assert_eq!(
            c.cached_bytes(),
            0,
            "oversized blob not cached on get either"
        );
    }

    #[tokio::test]
    async fn budget_zero_is_a_pure_passthrough() {
        // max_bytes = 0 disables the cache: put/get work (via the inner store) but nothing is ever cached.
        let mut c = CachingBlobStore::new(MemBlobStore::new(), 0);
        let bytes = blob(7, 100);
        let h = c.put(&bytes).await.unwrap();
        assert_eq!(c.cached_bytes(), 0);
        assert_eq!(c.cached_entries(), 0);
        assert_eq!(c.get(&h).await.unwrap().as_deref(), Some(&bytes[..]));
        assert_eq!(c.cached_bytes(), 0, "disabled cache never retains");
    }

    #[tokio::test]
    async fn re_putting_identical_bytes_does_not_double_count() {
        // Content-addressed: putting the same bytes twice is one entry, counted once (idempotent), recency
        // refreshed.
        let mut c = CachingBlobStore::new(MemBlobStore::new(), 1024);
        let bytes = blob(3, 100);
        let _ = c.put(&bytes).await.unwrap();
        let _ = c.put(&bytes).await.unwrap();
        assert_eq!(c.cached_entries(), 1, "same content = one entry");
        assert_eq!(c.cached_bytes(), 100, "counted once");
    }
}
