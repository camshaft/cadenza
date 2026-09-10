//! A size-capped on-disk [`BlobStore`] cache (`BoundedDiskCache`) — the middle tier of a
//! [`TieredBlobStore`](crate::TieredBlobStore): a local disk cache kept under a configured byte budget.
//!
//! It composes a [`DiskBlobStore`] for the actual (sharded, atomic) file I/O and adds an **in-memory
//! S3-FIFO index** for eviction — the same policy as the in-memory tier ([`MemoryCache`](crate::MemoryCache)),
//! but the *bytes live on disk* and only per-blob metadata (size + a 0–3 frequency + the small/main/ghost
//! queue membership) is held in memory. Eviction deletes files. The index is rebuilt by scanning the
//! directory on `open` (so cached DATA survives a restart; the eviction *state* — freq/queue order — resets,
//! which only affects future eviction choices, never correctness).
//!
//! It is a CACHE, so a `get` miss just falls through to a deeper tier. A write error from the backing disk
//! IS surfaced (it's a real I/O failure); eviction deletions are best-effort.

use crate::disk::DiskBlobStore;
use async_trait::async_trait;
use bytes::Bytes;
use cdz_platform::{BlobStore, BlobStoreError, Hash, HashTag};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;

type Digest = [u8; Hash::DIGEST_LEN];

/// Rebuild the `Blob`-tagged [`Hash`] for a digest (to address a file for deletion).
fn blob_hash(digest: &Digest) -> Hash {
    let mut bytes = [0u8; Hash::LEN];
    bytes[0] = HashTag::Blob as u8;
    bytes[1..].copy_from_slice(digest);
    Hash::from_bytes(bytes)
}

/// Per-blob metadata (the bytes are on disk).
struct Meta {
    size: usize,
    freq: u8,
}

const MAX_FREQ: u8 = 3;

/// The in-memory S3-FIFO index over the on-disk blobs. Mirrors [`MemoryCache`](crate::MemoryCache)'s policy
/// but tracks sizes (not bytes) and yields evicted digests for the caller to delete from disk.
struct Index {
    map: HashMap<Digest, Meta>,
    small: VecDeque<Digest>,
    main: VecDeque<Digest>,
    ghost: VecDeque<Digest>,
    ghost_set: HashSet<Digest>,
    small_bytes: usize,
    main_bytes: usize,
    capacity: usize,
    small_cap: usize,
    ghost_cap: usize,
}

impl Index {
    fn new(capacity: usize) -> Self {
        Self {
            map: HashMap::new(),
            small: VecDeque::new(),
            main: VecDeque::new(),
            ghost: VecDeque::new(),
            ghost_set: HashSet::new(),
            small_bytes: 0,
            main_bytes: 0,
            capacity,
            small_cap: (capacity / 10).max(1),
            ghost_cap: (capacity / 4096).clamp(16, 1 << 20),
        }
    }

    fn total(&self) -> usize {
        self.small_bytes + self.main_bytes
    }

    fn touch(&mut self, digest: &Digest) {
        if let Some(m) = self.map.get_mut(digest) {
            m.freq = (m.freq + 1).min(MAX_FREQ);
        }
    }

    /// Insert a newly-written blob, then run S3-FIFO eviction to fit the budget. Returns the digests whose
    /// files the caller must delete (evictions only — promotions stay on disk).
    fn insert_and_plan_evictions(&mut self, digest: Digest, size: usize) -> Vec<Digest> {
        if let Some(m) = self.map.get_mut(&digest) {
            m.freq = (m.freq + 1).min(MAX_FREQ); // already cached (content-addressed) — just warm it
            return Vec::new();
        }
        self.map.insert(digest, Meta { size, freq: 0 });
        if self.ghost_set.remove(&digest) {
            self.main.push_back(digest);
            self.main_bytes += size;
        } else {
            self.small.push_back(digest);
            self.small_bytes += size;
        }

        let mut victims = Vec::new();
        while self.total() > self.capacity {
            let from_small = self.small_bytes >= self.small_cap && !self.small.is_empty();
            let evicted = if from_small {
                self.evict_from_small()
            } else if !self.main.is_empty() {
                self.evict_from_main()
            } else if !self.small.is_empty() {
                self.evict_from_small()
            } else {
                break;
            };
            if let Some(d) = evicted {
                victims.push(d);
            }
        }
        victims
    }

    /// One step from `small`: promote a warm item (→ main), or evict a cold one. Returns the evicted digest
    /// (if any) for file deletion; `None` on a promotion (nothing freed from disk).
    fn evict_from_small(&mut self) -> Option<Digest> {
        while let Some(d) = self.small.pop_front() {
            let Some(m) = self.map.get_mut(&d) else {
                continue; // stale queue entry
            };
            let size = m.size;
            if m.freq > 0 {
                m.freq = 0;
                self.small_bytes -= size;
                self.main.push_back(d);
                self.main_bytes += size;
                return None;
            }
            self.map.remove(&d);
            self.small_bytes -= size;
            self.push_ghost(d);
            return Some(d);
        }
        None
    }

    /// One eviction from `main` (with a second chance for warm items). Returns the evicted digest.
    fn evict_from_main(&mut self) -> Option<Digest> {
        while let Some(d) = self.main.pop_front() {
            let Some(m) = self.map.get_mut(&d) else {
                continue;
            };
            if m.freq > 0 {
                m.freq -= 1;
                self.main.push_back(d);
                continue;
            }
            let size = m.size;
            self.map.remove(&d);
            self.main_bytes -= size;
            return Some(d);
        }
        None
    }

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
}

/// A `BlobStore` that caches blobs on disk under a byte budget, evicting by S3-FIFO. Composes a
/// [`DiskBlobStore`] for I/O.
pub struct BoundedDiskCache {
    disk: DiskBlobStore,
    index: Mutex<Index>,
    capacity: usize,
}

impl BoundedDiskCache {
    /// Open (creating if needed) a disk cache at `root` holding at most `capacity_bytes`. Scans any existing
    /// blobs into the index so cached data survives a restart.
    ///
    /// # Errors
    /// Propagates an I/O error opening or scanning the directory.
    pub fn open(
        root: impl Into<std::path::PathBuf>,
        capacity_bytes: usize,
    ) -> std::io::Result<Self> {
        let disk = DiskBlobStore::open(root)?;
        let mut index = Index::new(capacity_bytes);
        // Rebuild the index from the sharded tree: root/{c0}/{c1}/{base62-name}. Guard each level with
        // `is_dir` so a stray file (e.g. an interrupted `.tmp` write) never turns a `read_dir` into an error;
        // a leaf whose name isn't a valid base62 hash (a temp file) simply fails to parse and is skipped.
        for c0 in read_dir_entries(disk.root())? {
            if !c0.is_dir() {
                continue;
            }
            for c1 in read_dir_entries(&c0)? {
                if !c1.is_dir() {
                    continue;
                }
                for leaf in read_dir_entries(&c1)? {
                    let (Some(name), Ok(meta)) = (
                        leaf.file_name().and_then(|n| n.to_str()),
                        std::fs::metadata(&leaf),
                    ) else {
                        continue;
                    };
                    if let Ok(hash) = name.parse::<Hash>() {
                        let _ =
                            index.insert_and_plan_evictions(*hash.digest(), meta.len() as usize);
                    }
                }
            }
        }
        // Evict down to the budget if the on-disk set already exceeds it (handled by insert above as we go).
        Ok(Self {
            disk,
            index: Mutex::new(index),
            capacity: capacity_bytes,
        })
    }
}

/// List the entries (files/dirs) directly under `dir`; an absent dir yields nothing.
fn read_dir_entries(dir: &std::path::Path) -> std::io::Result<Vec<std::path::PathBuf>> {
    match std::fs::read_dir(dir) {
        Ok(rd) => Ok(rd.filter_map(|e| e.ok().map(|e| e.path())).collect()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(err),
    }
}

#[async_trait]
impl BlobStore for BoundedDiskCache {
    async fn put(&self, bytes: Bytes) -> Result<Hash, BlobStoreError> {
        // A blob larger than the whole budget can't be usefully cached — skip the write (the durable tier
        // holds it), just return its address.
        if bytes.len() > self.capacity {
            return Ok(Hash::of(HashTag::Blob, &bytes));
        }
        let size = bytes.len();
        let hash = self.disk.put(bytes).await?;
        // Plan evictions under the lock (sync), then delete the victim files outside it (no await held).
        let victims = {
            let mut index = self
                .index
                .lock()
                .expect("disk cache index lock not poisoned");
            index.insert_and_plan_evictions(*hash.digest(), size)
        };
        for digest in victims {
            let _ = self.disk.delete(&blob_hash(&digest)).await; // best-effort
        }
        Ok(hash)
    }

    async fn get(&self, hash: Hash) -> Result<Option<Bytes>, BlobStoreError> {
        let found = self.disk.get(hash).await?;
        if found.is_some() {
            self.index
                .lock()
                .expect("disk cache index lock not poisoned")
                .touch(hash.digest());
        }
        Ok(found)
    }

    async fn has(&self, hash: Hash) -> Result<bool, BlobStoreError> {
        self.disk.has(hash).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "cdz-cas-diskcache-{tag}-{}-{nanos}",
            std::process::id()
        ))
    }
    fn blob(n: usize) -> Bytes {
        let mut v = n.to_le_bytes().to_vec();
        v.resize(100, 0);
        Bytes::from(v)
    }

    #[tokio::test]
    async fn put_get_round_trips_and_reports_absence() {
        let dir = scratch("rt");
        let cache = BoundedDiskCache::open(&dir, 10_000).unwrap();
        let bytes = Bytes::from_static(b"cache on disk");
        let hash = cache.put(bytes.clone()).await.unwrap();
        assert_eq!(cache.get(hash).await.unwrap(), Some(bytes));
        assert!(cache.has(hash).await.unwrap());
        assert_eq!(
            cache.get(Hash::of(HashTag::Blob, b"absent")).await.unwrap(),
            None
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn evicts_to_stay_under_the_byte_budget() {
        let dir = scratch("budget");
        let cache = BoundedDiskCache::open(&dir, 1000).unwrap(); // ~10 blobs
        for i in 0..100 {
            cache.put(blob(i)).await.unwrap();
        }
        let total: usize = cache
            .index
            .lock()
            .unwrap()
            .map
            .values()
            .map(|m| m.size)
            .sum();
        assert!(total <= 1000, "cached bytes {total} within the 1000 budget");
        // The earliest cold blob was evicted from disk.
        assert_eq!(
            cache.get(Hash::of(HashTag::Blob, &blob(0))).await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn cached_data_survives_reopen() {
        let dir = scratch("reopen");
        let hash = {
            let cache = BoundedDiskCache::open(&dir, 10_000).unwrap();
            cache.put(Bytes::from_static(b"persist me")).await.unwrap()
        };
        // A fresh cache on the same dir rebuilds its index from the files and still serves the blob.
        let reopened = BoundedDiskCache::open(&dir, 10_000).unwrap();
        assert_eq!(
            reopened.get(hash).await.unwrap(),
            Some(Bytes::from_static(b"persist me"))
        );
        assert_eq!(reopened.index.lock().unwrap().map.len(), 1, "index rebuilt");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn an_oversized_blob_is_not_cached() {
        let dir = scratch("oversized");
        let cache = BoundedDiskCache::open(&dir, 64).unwrap();
        let big = Bytes::from(vec![7u8; 1000]);
        let hash = cache.put(big).await.unwrap();
        assert_eq!(cache.get(hash).await.unwrap(), None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
