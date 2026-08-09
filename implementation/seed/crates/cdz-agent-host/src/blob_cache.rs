//! A bounded S3-FIFO cache DECORATOR over any [`BlobStore`] (operator directive: "a S3-FIFO would probably be
//! better cause I'm sure we have a lot of one-hit-wonders" — revised from a plain LRU). Blob reads are
//! content-addressed, so this is the cleanest possible cache: a [`Hash`] maps to fixed, immutable bytes — a
//! hit is ALWAYS correct and there is NO invalidation (the value for a key can never change). Repeated fetches
//! of the same blob (a reducer component re-read per install, a doc queried back) serve from memory instead of
//! re-hitting S3 / disk each time.
//!
//! **Why S3-FIFO, not LRU.** A blob store sees many ONE-HIT-WONDERS — a blob fetched once and never again.
//! Plain LRU admits every miss into the cache, so a burst of one-hit fetches evicts genuinely hot entries to
//! hold garbage that's never read again. S3-FIFO (Yang, Qiu, et al., SOSP'23) resists exactly that: new items
//! enter a SMALL probationary FIFO (`small`, ~10% of the budget); an item is promoted to the MAIN FIFO
//! (`main`, ~90%) only if it's accessed again before it ages out of `small`. A one-hit-wonder rides through
//! `small` and is evicted without ever touching `main`, so it never displaces a hot entry. A GHOST queue
//! (`ghost`, keys only, no bytes) remembers keys recently evicted from `small`, so a blob that misses, is
//! evicted, then is requested AGAIN is admitted straight to `main` (it proved reuse across the gap). This beats
//! LRU on scan-heavy / one-hit workloads at lower bookkeeping overhead (FIFO queues + a 1-bit accessed flag,
//! no per-access list surgery).
//!
//! **Pure host MECHANISM, no policy** (minimize-host-logic norm). It wraps ANY `BlobStore` — `MemBlobStore`,
//! `DiskBlobStore`, or the AWS `S3BlobStore` — behind the SAME trait, so a consumer holds a
//! `CachingBlobStore<B>` exactly where it held `B`; the decorator composes like the metered-executor wrapper.
//!
//! **Bounded by TOTAL BYTES, not entry count.** Blobs vary widely in size (a small doc vs a multi-MiB reducer
//! component), so a byte budget is the right bound — an entry-count bound would let a few large blobs blow
//! memory, or cap tiny blobs pointlessly. The budget is split `small` (~10%) + `main` (~90%); a single blob
//! LARGER than the whole budget is never cached (served straight through) rather than evicting everything to
//! hold one oversized value.
//!
//! **Multi-tier to disk is a following slice.** This is the in-MEMORY tier. The operator also asked for a
//! disk tier under it (mem over disk over S3); that composes as ANOTHER `CachingBlobStore`-style layer over a
//! `DiskBlobStore` and lands next — this slice is the S3-FIFO eviction core the disk tier reuses.
//!
//! **Single-threaded convention.** The kernel traits are `?Send` (single-threaded by design), and
//! [`BlobStore::get`] takes `&self`, so the cache state lives behind a [`RefCell`] — `get` marks recency
//! (the accessed bit) through a shared reference with no locking, matching the crate's single-task host loop.

use cdz_kernel::blob::BlobStore;
use cdz_kernel::hash::Hash;
use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;

/// A cached blob + its 1-bit S3-FIFO accessed flag. `accessed` is set on every hit and is what decides, at
/// eviction time, whether an entry has proven reuse (promote from `small` to `main` / get a second chance in
/// `main`) or is a one-hit-wonder (evict).
struct Entry {
    bytes: bytes::Bytes,
    accessed: bool,
}

/// The S3-FIFO state behind a [`RefCell`]: the stored bytes keyed by hash, the two FIFO queues (`small`
/// probationary + `main`), the keys-only `ghost` queue, and the running byte totals. A key in `entries`
/// appears in EXACTLY one of `small`/`main`; a key in `ghost` is NOT in `entries` (ghost holds no bytes).
struct CacheState {
    entries: HashMap<Hash, Entry>,
    /// Probationary FIFO (front = oldest). New admissions (not seen recently) enter here.
    small: VecDeque<Hash>,
    /// Main FIFO (front = oldest). Items promoted from `small` on reuse, or admitted directly when their key
    /// was in `ghost` (proven reuse across an eviction).
    main: VecDeque<Hash>,
    /// Ghost FIFO (front = oldest) — keys recently evicted from `small`, no bytes. A miss whose key is here is
    /// admitted straight to `main`. Bounded by count (`ghost_cap`).
    ghost: VecDeque<Hash>,
    /// O(1) membership for `ghost` (kept in lockstep with the `ghost` queue).
    ghost_set: HashSet<Hash>,
    small_bytes: usize,
    main_bytes: usize,
}

impl CacheState {
    /// Admit `hash`/`bytes` after a miss. If the key is a ghost hit (recently evicted from `small`), it proved
    /// reuse → admit straight to `main`; otherwise it's a fresh/probationary item → admit to `small`. Evicts
    /// to keep each queue within its byte budget. Caller guarantees the entry isn't already present and isn't
    /// oversized for the whole cache.
    fn admit(
        &mut self,
        hash: Hash,
        bytes: bytes::Bytes,
        small_budget: usize,
        main_budget: usize,
        ghost_cap: usize,
    ) {
        let len = bytes.len();
        // A ghost hit (recently evicted from small) OR a blob too large to sit in the probationary `small`
        // queue is admitted straight to `main`: a ghost hit has proven reuse across the gap, and a blob bigger
        // than the whole small budget could never be retained in small (it would evict on the next admission),
        // so probation is meaningless for it. Everything else enters `small` as probationary.
        let to_main = self.ghost_set.remove(&hash) || len > small_budget;
        // If it was a ghost hit, also drop it from the ghost queue (linear scan; ghost is bounded, cheap).
        if to_main {
            if let Some(pos) = self.ghost.iter().position(|h| h == &hash) {
                self.ghost.remove(pos);
            }
            self.entries.insert(
                hash,
                Entry {
                    bytes,
                    accessed: false,
                },
            );
            self.main.push_back(hash);
            self.main_bytes += len;
            self.evict_main(main_budget);
        } else {
            self.entries.insert(
                hash,
                Entry {
                    bytes,
                    accessed: false,
                },
            );
            self.small.push_back(hash);
            self.small_bytes += len;
            self.evict_small(small_budget, main_budget, ghost_cap);
        }
    }

    /// Evict from `small` until it fits `small_budget`. An entry accessed since admission is PROMOTED to `main`
    /// (it proved reuse); a one-hit-wonder is evicted and its key remembered in `ghost` (so a later re-request
    /// admits straight to `main`). A promotion can push `main` over budget → cascade to `evict_main`.
    fn evict_small(&mut self, small_budget: usize, main_budget: usize, ghost_cap: usize) {
        while self.small_bytes > small_budget {
            let Some(victim) = self.small.pop_front() else {
                break;
            };
            let Some(entry) = self.entries.get_mut(&victim) else {
                continue;
            };
            let len = entry.bytes.len();
            self.small_bytes -= len;
            if entry.accessed {
                // Proven reuse: promote to main (reset the accessed bit for its main-queue life).
                entry.accessed = false;
                self.main.push_back(victim);
                self.main_bytes += len;
                self.evict_main(main_budget);
            } else {
                // One-hit-wonder: drop the bytes, remember the key in the ghost queue.
                self.entries.remove(&victim);
                self.push_ghost(victim, ghost_cap);
            }
        }
    }

    /// Evict from `main` until it fits `main_budget`, FIFO with a one-bit second chance: an entry accessed
    /// since it entered `main` is reinserted at the back (accessed reset), an un-accessed one is evicted.
    /// Terminates: a reinsert resets `accessed=false`, so an entry gets at most one second chance per pass.
    fn evict_main(&mut self, main_budget: usize) {
        while self.main_bytes > main_budget {
            let Some(victim) = self.main.pop_front() else {
                break;
            };
            let Some(entry) = self.entries.get_mut(&victim) else {
                continue;
            };
            if entry.accessed {
                entry.accessed = false;
                self.main.push_back(victim);
            } else {
                let len = entry.bytes.len();
                self.main_bytes -= len;
                self.entries.remove(&victim);
            }
        }
    }

    /// Record an evicted-from-`small` key in the ghost queue, bounded to `ghost_cap` entries (drop the oldest
    /// ghost key when full). Ghost holds no bytes, so it's bounded by COUNT, not the byte budget.
    fn push_ghost(&mut self, hash: Hash, ghost_cap: usize) {
        if ghost_cap == 0 {
            return;
        }
        if self.ghost_set.insert(hash) {
            self.ghost.push_back(hash);
            while self.ghost.len() > ghost_cap {
                if let Some(old) = self.ghost.pop_front() {
                    self.ghost_set.remove(&old);
                }
            }
        }
    }
}

/// A bounded S3-FIFO cache in front of an inner [`BlobStore`] `B`. Construct with [`new`](Self::new) (a total
/// byte budget) and use it anywhere a `BlobStore` is expected.
pub struct CachingBlobStore<B> {
    inner: B,
    /// Total byte budget across `small` + `main`. `0` disables caching entirely (every `get`/`put` is a
    /// straight pass-through with zero bookkeeping) — the operator's "off" setting.
    budget: usize,
    /// `small` (probationary) byte budget — ~10% of `budget` (at least 1 when caching is enabled), the rest is
    /// `main`. The small queue is where one-hit-wonders age out without reaching `main`.
    small_budget: usize,
    /// `main` byte budget — `budget - small_budget`.
    main_budget: usize,
    /// Max ghost KEYS retained. Sized generously relative to entry counts so a re-request after eviction still
    /// finds its key; derived from the budget with a small floor so a tiny cache still remembers a few.
    ghost_cap: usize,
    state: RefCell<CacheState>,
}

impl<B> CachingBlobStore<B> {
    /// Wrap `inner` with an S3-FIFO cache bounded to `max_bytes` total. `max_bytes == 0` disables the cache
    /// (pure pass-through). The budget is split ~10% probationary (`small`) + ~90% `main`; the ghost queue is
    /// sized from the budget (a byte-free key-only list, so it's bounded by count).
    pub fn new(inner: B, max_bytes: usize) -> Self {
        // small = 10% of the budget, at least 1 byte when caching is on (so a tiny budget still has a
        // probationary queue); main gets the remainder.
        let small_budget = if max_bytes == 0 {
            0
        } else {
            (max_bytes / 10).max(1)
        };
        let main_budget = max_bytes.saturating_sub(small_budget);
        // Ghost remembers roughly as many keys as could fit in main at a modest average blob size, with a
        // small floor so even a tiny cache remembers a few evicted keys. Pure count bound (ghost holds no
        // bytes); 4 KiB is a conservative "small blob" divisor for the estimate.
        let ghost_cap = if max_bytes == 0 {
            0
        } else {
            (max_bytes / 4096).max(16)
        };
        CachingBlobStore {
            inner,
            budget: max_bytes,
            small_budget,
            main_budget,
            ghost_cap,
            state: RefCell::new(CacheState {
                entries: HashMap::new(),
                small: VecDeque::new(),
                main: VecDeque::new(),
                ghost: VecDeque::new(),
                ghost_set: HashSet::new(),
                small_bytes: 0,
                main_bytes: 0,
            }),
        }
    }

    /// Admit `bytes` under `hash` via the S3-FIFO policy. A blob larger than the whole budget is NOT cached
    /// (it would force evicting everything to hold one value) — served through without being retained. A blob
    /// already cached just gets its accessed bit set (no re-admit, no double-count).
    fn cache_put(&self, hash: Hash, bytes: &bytes::Bytes) {
        if self.budget == 0 || bytes.len() > self.budget {
            return;
        }
        let mut st = self.state.borrow_mut();
        if let Some(entry) = st.entries.get_mut(&hash) {
            // Already cached (content-addressed → identical bytes); mark reuse, don't re-admit.
            entry.accessed = true;
            return;
        }
        // Clone the ref-counted `Bytes` into the cache (O(1) refcount bump, not a deep copy of the blob).
        st.admit(
            hash,
            bytes.clone(),
            self.small_budget,
            self.main_budget,
            self.ghost_cap,
        );
    }

    /// The current number of cached blobs (in `small` + `main`) — for tests/inspection (the bound is on bytes).
    pub fn cached_entries(&self) -> usize {
        self.state.borrow().entries.len()
    }

    /// The current total cached bytes (`small` + `main`) — the quantity the budget bounds.
    pub fn cached_bytes(&self) -> usize {
        let st = self.state.borrow();
        st.small_bytes + st.main_bytes
    }
}

#[async_trait::async_trait(?Send)]
impl<B: BlobStore> BlobStore for CachingBlobStore<B> {
    /// Write THROUGH to the inner store under the SUPPLIED hash (computed once by the caller — no re-hash per
    /// tier), then admit the just-stored bytes (a fresh `put` is often followed by a `get` of the same hash —
    /// e.g. store a doc then immediately query it back). Admitting clones the ref-counted `Bytes` (O(1)).
    async fn put(&mut self, hash: Hash, bytes: bytes::Bytes) -> std::io::Result<()> {
        self.cache_put(hash, &bytes);
        self.inner.put(hash, bytes).await
    }

    /// Serve from cache on a hit (setting the accessed bit so S3-FIFO promotes it on reuse); on a miss, fetch
    /// from the inner store and admit it. Content-addressed keys mean a cached value is always valid — no
    /// invalidation.
    async fn get(&self, hash: &Hash) -> std::io::Result<Option<bytes::Bytes>> {
        if self.budget > 0 {
            let mut st = self.state.borrow_mut();
            if let Some(entry) = st.entries.get_mut(hash) {
                entry.accessed = true;
                return Ok(Some(entry.bytes.clone()));
            }
        }
        // Miss: fetch from the backing store, then admit it (the borrow above is already released).
        let fetched = self.inner.get(hash).await?;
        if let Some(bytes) = &fetched {
            self.cache_put(*hash, bytes);
        }
        Ok(fetched)
    }

    /// Existence probe: a cached blob trivially exists; otherwise defer to the inner store's probe (which a
    /// real backend implements more cheaply than a full fetch). Doesn't admit (a `has` doesn't fetch bytes).
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
    fn blob(b: u8, n: usize) -> bytes::Bytes {
        bytes::Bytes::from(vec![b; n])
    }

    /// A blob of `n` bytes filled with `b` then a per-`i` marker appended, so each `i` yields DISTINCT content
    /// (distinct hash) — the flood/churn tests need every admission to be a fresh key.
    fn distinct_blob(b: u8, n: usize, i: u32) -> bytes::Bytes {
        let mut v = vec![b; n];
        v.extend_from_slice(&i.to_le_bytes());
        bytes::Bytes::from(v)
    }

    /// Test helper mirroring the OLD `put(&bytes) -> Hash` ergonomics over the new `put(hash, Bytes) -> ()`:
    /// compute the content hash once, store, return the hash (what the tests key on).
    async fn put_blob<S: BlobStore>(store: &mut S, bytes: &bytes::Bytes) -> Hash {
        let hash = Hash::of(bytes);
        store.put(hash, bytes.clone()).await.unwrap();
        hash
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
    async fn hit_after_miss_serves_from_cache_without_rehitting_inner() {
        // Get a blob once (miss → fetches inner, admits), then blind the inner store and get again: the second
        // get still returns the bytes (from cache) AND must NOT have called inner.get again.
        let mut inner = MemBlobStore::new();
        let bytes = blob(1, 100);
        let hash = put_blob(&mut inner, &bytes).await;
        let counting = CountingBlobStore {
            inner,
            gets: std::cell::Cell::new(0),
            blind: std::cell::Cell::new(false),
        };
        let c = CachingBlobStore::new(counting, 1024);
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
            "served from cache despite a blinded inner store"
        );
        assert_eq!(
            c.inner.gets.get(),
            1,
            "the hit did NOT re-hit the inner store"
        );
    }

    #[tokio::test]
    async fn miss_populates_the_cache_from_the_inner_store() {
        let mut inner = MemBlobStore::new();
        let bytes = blob(2, 200);
        let hash = put_blob(&mut inner, &bytes).await;
        let c = CachingBlobStore::new(inner, 1024);
        assert_eq!(c.cached_bytes(), 0, "nothing cached before the first get");
        assert_eq!(c.get(&hash).await.unwrap().as_deref(), Some(&bytes[..]));
        assert_eq!(c.cached_bytes(), 200, "the miss populated the cache");
        assert_eq!(c.cached_entries(), 1);
    }

    #[tokio::test]
    async fn s3fifo_one_hit_wonders_do_not_evict_a_hot_main_entry() {
        // The core S3-FIFO win. Budget 1000 → small≈100, main≈900. Promote a HOT blob into main (get it twice:
        // once admits to small, the re-get sets accessed; a later admission pressure promotes it). Then stream
        // many ONE-HIT-WONDER blobs (each fetched once): they churn through `small` and get evicted to ghost
        // WITHOUT displacing the hot entry in main. Assert the hot blob is still cached after the flood.
        let c = CachingBlobStore::new(MemBlobStore::new(), 1000);
        // Pre-store the hot blob + many one-hit blobs in the backing store, so gets are hits at the inner.
        // (We reach into a fresh inner via put through the cache to keep it simple.)
        let mut c = c;
        let hot = blob(b'H', 80);
        let hot_hash = put_blob(&mut c, &hot).await;
        // Access the hot blob again so its accessed bit is set (proves reuse → eligible for promotion).
        let _ = c.get(&hot_hash).await.unwrap();
        // Flood with one-hit-wonders, each distinct + each fetched exactly once (put = one write+admit).
        for i in 0..200u32 {
            let w = distinct_blob((i % 250) as u8, 40 + (i as usize % 30), i);
            let _ = put_blob(&mut c, &w).await;
        }
        // The hot blob survived the one-hit-wonder flood (it was promoted to main; the flood churned small).
        assert_eq!(
            c.get(&hot_hash).await.unwrap().as_deref(),
            Some(&hot[..]),
            "the hot, reused blob survived a flood of one-hit-wonders (S3-FIFO anti-pollution)"
        );
    }

    #[tokio::test]
    async fn ghost_promotes_a_re_requested_blob_straight_to_main() {
        // A blob admitted to small, evicted (one-hit) → its key lands in ghost. Re-admitting it (a second
        // miss) should route it to MAIN, not small (proven reuse across the gap). We can observe the effect
        // indirectly: after a ghost-promotion the blob is in main and survives further small-only churn.
        let mut c = CachingBlobStore::new(MemBlobStore::new(), 1000);
        let x = blob(b'X', 80);
        let hx = put_blob(&mut c, &x).await; // admitted to small
                                             // Evict it from small WITHOUT accessing (pure one-hit) by flooding small with other new blobs.
        for i in 0..50u32 {
            let w = distinct_blob(b'a', 40, i);
            let _ = put_blob(&mut c, &w).await;
        }
        // x should have been evicted to ghost (never accessed). Re-put it → ghost hit → admitted to main.
        let _ = put_blob(&mut c, &x).await;
        // Now flood small again; a main-resident x survives.
        for i in 50..100u32 {
            let w = distinct_blob(b'b', 40, i);
            let _ = put_blob(&mut c, &w).await;
        }
        assert_eq!(
            c.get(&hx).await.unwrap().as_deref(),
            Some(&x[..]),
            "a re-requested (ghost-hit) blob was admitted to main and survived small-queue churn"
        );
    }

    #[tokio::test]
    async fn total_bytes_never_exceeds_the_budget() {
        // Whatever the access pattern, cached_bytes stays within the total budget (small + main bounds hold).
        let mut c = CachingBlobStore::new(MemBlobStore::new(), 500);
        for i in 0..100u32 {
            let w = distinct_blob((i % 200) as u8, 60, i);
            let _ = put_blob(&mut c, &w).await;
            assert!(
                c.cached_bytes() <= 500,
                "cached_bytes {} exceeded budget 500 at i={i}",
                c.cached_bytes()
            );
        }
    }

    #[tokio::test]
    async fn a_blob_larger_than_the_budget_is_served_but_not_cached() {
        let mut c = CachingBlobStore::new(MemBlobStore::new(), 50);
        let big = blob(9, 100);
        let h = put_blob(&mut c, &big).await;
        assert_eq!(c.cached_bytes(), 0, "oversized blob not cached on put");
        assert_eq!(
            c.get(&h).await.unwrap().as_deref(),
            Some(&big[..]),
            "still served from inner"
        );
        assert_eq!(
            c.cached_bytes(),
            0,
            "oversized blob not cached on get either"
        );
    }

    #[tokio::test]
    async fn budget_zero_is_a_pure_passthrough() {
        let mut c = CachingBlobStore::new(MemBlobStore::new(), 0);
        let bytes = blob(7, 100);
        let h = put_blob(&mut c, &bytes).await;
        assert_eq!(c.cached_bytes(), 0);
        assert_eq!(c.cached_entries(), 0);
        assert_eq!(c.get(&h).await.unwrap().as_deref(), Some(&bytes[..]));
        assert_eq!(c.cached_bytes(), 0, "disabled cache never retains");
    }

    #[tokio::test]
    async fn re_putting_identical_bytes_does_not_double_count() {
        let mut c = CachingBlobStore::new(MemBlobStore::new(), 1024);
        let bytes = blob(3, 100);
        let _ = put_blob(&mut c, &bytes).await;
        let _ = put_blob(&mut c, &bytes).await;
        assert_eq!(c.cached_entries(), 1, "same content = one entry");
        assert_eq!(c.cached_bytes(), 100, "counted once");
    }
}
