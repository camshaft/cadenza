//! Content-addressable blob store (CAS) — the `hash → bytes` fetch layer (§4 blob store).
//!
//! The kernel already has content-ADDRESSING (`hash.rs`) and content-addressed events + KV-root
//! hashes, but until now nothing maps a hash back to its bytes. This is that layer: `put(bytes) →
//! Hash` (store, keyed by content hash) and `get(&Hash) → Option<bytes>` (fetch). It's the §4 blob
//! store the design leans on for large payloads (a KV/event carries a hash; the bytes live here) AND
//! the prerequisite for component-dependency linking (operator directive §21b): a Cadenza reducer
//! component references the value-heap runtime component by hash, so the kernel must resolve those
//! deps from CAS before it can compose + run the reducer.
//!
//! **Content-addressed integrity (free):** since the key IS `Hash::of(bytes)`, the store is
//! self-verifying — `put` computes the hash, and a `get` can re-verify the returned bytes hash to the
//! requested key (a disk backend does this, so a corrupted/tampered blob file is caught, not served).
//! Immutable by construction: the same bytes always map to the same key; writing an existing key is a
//! no-op (idempotent), never a mutation.
//!
//! **Backend is a trait** (matches §19b: host storage backends are swappable traits, not baked in) —
//! [`MemBlobStore`] for tests/single-process, [`DiskBlobStore`] for durability. A future
//! network/outpost-fetching backend (§12) implements the same trait.

use crate::hash::Hash;
use bytes::Bytes;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Process-global counter for unique temp-file names on disk puts. Combined with the pid it makes every
/// in-flight `put`'s temp path distinct, so concurrent puts (even of the SAME content → same final
/// name) never share a temp file and interleave into a torn write. (Uniqueness only — unlike a metric
/// counter, sharing this across threads/tests is correct; it just needs to never hand out the same
/// value twice in a process.)
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// A content-addressable blob store: put bytes (keyed by their content hash), get them back by hash.
/// Backends are swappable (in-memory, disk, later network). Errors are backend I/O; a missing blob is
/// `Ok(None)`, not an error (absence is a normal answer — the caller decides if it's fatal).
/// ASYNC (operator directive): the CAS backend must be `async` so a remote store (S3/Dynamo) drops in
/// behind the same trait without a sync→async churn — content-addressed blobs are the obvious thing to
/// put in S3 ("especially this one"). The local `MemBlobStore`/`DiskBlobStore` backends satisfy it with
/// no real await (in-memory / `std::fs`); a network backend awaits its transport. `?Send` because the
/// kernel is single-threaded by design (matches the Executor/Reducer/Authorize traits).
#[async_trait::async_trait(?Send)]
pub trait BlobStore {
    /// Store `bytes` under their content `hash` (the key). The hash is SUPPLIED, not recomputed
    /// (operator directive): a blob is written through N cache tiers (mem-s3fifo → disk → S3) and the
    /// content hash is a pure function of the bytes, so it is computed ONCE at the top and threaded —
    /// recomputing blake3 per backend/tier is wasted CPU. `bytes` is [`bytes::Bytes`] (cheaply-clonable:
    /// a clone is an O(1) refcount bump, never a deep copy, as the blob threads the tiers). Idempotent:
    /// storing a key already present is a no-op (content-addressed → the key can't hold different
    /// content). A backend MAY verify `Hash::of(&bytes) == hash` defensively; it MUST NOT recompute the
    /// key from the bytes and store under that (the caller's supplied hash IS the key — a disk backend's
    /// `get` re-verify catches a lying caller).
    async fn put(&mut self, hash: Hash, bytes: Bytes) -> std::io::Result<()>;

    /// Fetch the bytes for `hash`, or `None` if absent. Returns [`bytes::Bytes`] (cheaply-clonable — a
    /// blob cached across tiers hands out O(1) refcount clones, not deep copies). A backend that can
    /// verify integrity SHOULD (re-hash the bytes, refuse a mismatch) — content-addressing makes
    /// tamper-detection free.
    async fn get(&self, hash: &Hash) -> std::io::Result<Option<Bytes>>;

    /// Is a blob present? A backend SHOULD override this with a real existence probe (a disk backend
    /// stats the file, an object store issues a HEAD) — the check is only as cheap as the impl makes it.
    /// The default below is a correctness fallback that performs a FULL `get`, so an un-overridden backend
    /// pays the fetch cost; override it wherever a probe is meaningfully cheaper than a fetch.
    async fn has(&self, hash: &Hash) -> std::io::Result<bool> {
        Ok(self.get(hash).await?.is_some())
    }
}

/// In-memory blob store — for tests and single-process use where durability isn't needed.
#[derive(Default)]
pub struct MemBlobStore {
    blobs: HashMap<Hash, Bytes>,
}

impl MemBlobStore {
    pub fn new() -> Self {
        MemBlobStore {
            blobs: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.blobs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.blobs.is_empty()
    }
}

#[async_trait::async_trait(?Send)]
impl BlobStore for MemBlobStore {
    async fn put(&mut self, hash: Hash, bytes: Bytes) -> std::io::Result<()> {
        // Idempotent: only insert if absent (content-addressed → an existing key holds identical bytes).
        // The `hash` is the caller's pre-computed key (computed once at the top); we do NOT recompute it.
        self.blobs.entry(hash).or_insert(bytes);
        Ok(())
    }

    async fn get(&self, hash: &Hash) -> std::io::Result<Option<Bytes>> {
        // O(1) refcount clone (Bytes), not a deep copy.
        Ok(self.blobs.get(hash).cloned())
    }

    async fn has(&self, hash: &Hash) -> std::io::Result<bool> {
        Ok(self.blobs.contains_key(hash))
    }
}

/// Disk-backed blob store — one file per blob, named by the hex of its content hash, under a root dir.
/// Durable + self-verifying: `get` re-hashes the file's bytes and refuses to serve a mismatch (a
/// corrupted/tampered blob file is caught, not returned as if valid).
pub struct DiskBlobStore {
    root: PathBuf,
}

impl DiskBlobStore {
    /// Open (creating the root dir if absent) a disk blob store rooted at `root`.
    pub fn open(root: impl AsRef<Path>) -> std::io::Result<Self> {
        let root = root.as_ref().to_path_buf();
        std::fs::create_dir_all(&root)?;
        Ok(DiskBlobStore { root })
    }

    /// The SHARDED directory a blob lives in: `root/{hex[0..2]}/{hex[2..4]}` (operator review, S3-blob PR
    /// #2548 — "same goes for the filesystem"). Fanning the content-hash keyspace across nested dirs avoids a
    /// single giant flat directory (filesystem perf degrades + inode/dir-entry limits bite at scale — the
    /// standard CAS layout, cf. git's `.git/objects/{hh}/`). The blob's file basename is the REST of the hex
    /// (`hex[4..]`). Content-addressed, so key→path derivation is the backend's private business — the
    /// `BlobStore` trait + all callers are unaffected. A blake3 hex is 64 chars, so the two 2-char shard
    /// segments always exist.
    fn shard_dir(&self, hex: &str) -> PathBuf {
        self.root.join(&hex[0..2]).join(&hex[2..4])
    }

    fn path_for(&self, hash: &Hash) -> PathBuf {
        let hex = hash.to_hex();
        self.shard_dir(&hex).join(&hex[4..])
    }
}

#[async_trait::async_trait(?Send)]
impl BlobStore for DiskBlobStore {
    async fn put(&mut self, hash: Hash, bytes: Bytes) -> std::io::Result<()> {
        // `hash` is the caller's pre-computed content key (computed ONCE at the top, threaded through the
        // tiers — operator directive: no per-backend recompute). We write under it; the `get` self-verify
        // catches a mismatched caller. `bytes: Bytes` is cheaply-clonable (no deep copy to reach here).
        let path = self.path_for(&hash);
        // Idempotent — BUT don't blindly trust an existing file (PR#1010): a blob that bit-rotted or was
        // tampered on disk still sits under the right name, and skipping the write on mere existence
        // would leave that corruption in place forever while reporting success. Verify the existing
        // file's bytes actually hash to the key; only then treat the put as a no-op. If it's missing or
        // corrupt, fall through and (re)write it — a put is the moment we can heal a bad blob.
        match std::fs::read(&path) {
            Ok(existing) if Hash::of(&existing) == hash => return Ok(()),
            Ok(_) => { /* corrupt on disk — fall through to rewrite the good bytes */ }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => { /* not present — write it */ }
            Err(e) => return Err(e),
        }
        // Write to a UNIQUE temp file + atomic rename (PR#1011): the temp name carries a process-unique
        // (pid, seq) suffix, so two concurrent puts of the SAME content don't share a `{hash}.tmp` path
        // and interleave into a torn write that then renames a corrupt file into place. Each put writes
        // its own temp fully, then renames — the final blob is always a complete, single writer's bytes.
        let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        // The tmp file lives in the SAME shard dir as the final path, so the rename below is intra-directory
        // (truly atomic on every filesystem — a cross-dir rename can be non-atomic). Create the shard dir
        // first (the sharded layout, unlike the old flat root, needs its {hh}/{hh} dirs to exist).
        let hex = hash.to_hex();
        let shard = self.shard_dir(&hex);
        std::fs::create_dir_all(&shard)?;
        let tmp = shard.join(format!("{}.{}.{}.tmp", &hex[4..], std::process::id(), seq));
        std::fs::write(&tmp, &bytes)?;
        // Rename tmp → final. POSIX rename atomically REPLACES an existing target, but Windows rename
        // FAILS if the target exists — so the corrupt-rewrite path (target present but bad bytes) would
        // leave the corruption UNHEALED on Windows (Copilot PR#1016, same rename-over-existing class as
        // PR#903/#929). Recover ONLY from the specific "destination already exists" error by removing the
        // target + retrying once.
        //
        // WARNING: It must be THAT error kind only (Copilot PR#1018 DATA-LOSS regression): the earlier fallback
        // removed `path` on ANY rename error as long as the target existed — but rename also fails for
        // permission/IO/cross-filesystem reasons, so it would DELETE A VALID BLOB and then fail. On any
        // error that isn't AlreadyExists, leave `path` untouched, clean up tmp, and surface the error.
        if let Err(e) = std::fs::rename(&tmp, &path) {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                // Windows dest-exists: remove the (corrupt/stale) target and retry the rename once.
                if let Err(e2) =
                    std::fs::remove_file(&path).and_then(|()| std::fs::rename(&tmp, &path))
                {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(e2);
                }
            } else {
                // Permission/IO/cross-fs/etc. — do NOT touch `path` (it may be a valid blob). Just
                // clean up our temp and report.
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
        }
        Ok(())
    }

    async fn get(&self, hash: &Hash) -> std::io::Result<Option<Bytes>> {
        let path = self.path_for(hash);
        match std::fs::read(&path) {
            Ok(bytes) => {
                // Self-verify (integrity is free with content-addressing): the file's bytes MUST hash
                // to the requested key, or it's corrupt/tampered — refuse to serve it.
                if Hash::of(&bytes) == *hash {
                    Ok(Some(Bytes::from(bytes)))
                } else {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!(
                            "blob {} failed content-hash verification (corrupt/tampered)",
                            hash
                        ),
                    ))
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    async fn has(&self, hash: &Hash) -> std::io::Result<bool> {
        Ok(self.path_for(hash).exists())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test helper: compute the content hash ONCE (as a real caller does — put no longer computes it) +
    /// store the bytes under it, returning the hash for assertions. Mirrors the production compute-once
    /// pattern.
    async fn put_blob<S: BlobStore>(store: &mut S, bytes: &'static [u8]) -> Hash {
        let hash = Hash::of(bytes);
        store.put(hash, Bytes::from_static(bytes)).await.unwrap();
        hash
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("cdz-kernel-blob-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&p);
        p
    }

    /// The SHARDED on-disk path a DiskBlobStore blob lives at: `root/{hex[0..2]}/{hex[2..4]}/{hex[4..]}`
    /// (mirrors DiskBlobStore::path_for — tests that inspect/tamper the raw file must use the sharded layout).
    fn sharded_path(root: &Path, hash: &Hash) -> PathBuf {
        let hex = hash.to_hex();
        root.join(&hex[0..2]).join(&hex[2..4]).join(&hex[4..])
    }

    #[tokio::test(flavor = "current_thread")]
    async fn disk_store_shards_the_content_hash_keyspace() {
        // Operator review (PR #2548): the content-hash key/path is SHARDED {hh}/{hh}/{rest}, not flat — so a
        // blob lands at root/{hex[0..2]}/{hex[2..4]}/{hex[4..]}, the standard CAS fan-out (no giant flat dir).
        let dir = temp_dir("shard");
        let mut store = DiskBlobStore::open(&dir).unwrap();
        let h = put_blob(&mut store, b"sharded blob").await;
        let hex = h.to_hex();
        // The blob is at the sharded path, and NOT at the old flat path.
        assert!(
            sharded_path(&dir, &h).is_file(),
            "blob lives at root/{{hh}}/{{hh}}/{{rest}}"
        );
        assert!(
            !dir.join(&hex).is_file(),
            "blob is NOT at the old flat root/{{hex}} path"
        );
        // The two shard segments are the first two hex byte-pairs; the basename is the rest.
        assert!(dir.join(&hex[0..2]).join(&hex[2..4]).is_dir());
        // Round-trips through the trait (get finds it via the same sharded derivation).
        assert_eq!(
            store.get(&h).await.unwrap().as_deref(),
            Some(&b"sharded blob"[..])
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    async fn round_trip<S: BlobStore>(store: &mut S) {
        let h = put_blob(store, b"hello blob").await;
        // The key is the content hash.
        assert_eq!(h, Hash::of(b"hello blob"));
        assert_eq!(
            store.get(&h).await.unwrap().as_deref(),
            Some(&b"hello blob"[..])
        );
        assert!(store.has(&h).await.unwrap());
        // Absent blob → None (not an error).
        assert_eq!(store.get(&Hash::of(b"never stored")).await.unwrap(), None);
        assert!(!store.has(&Hash::of(b"never stored")).await.unwrap());
        // Idempotent put: same bytes → same hash, no duplicate.
        let h2 = put_blob(store, b"hello blob").await;
        assert_eq!(h, h2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn mem_store_round_trips() {
        let mut store = MemBlobStore::new();
        round_trip(&mut store).await;
        // idempotent put didn't grow the store beyond the distinct blobs stored.
        assert_eq!(store.len(), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn disk_store_round_trips_and_persists() {
        let dir = temp_dir("roundtrip");
        {
            let mut store = DiskBlobStore::open(&dir).unwrap();
            round_trip(&mut store).await;
        }
        // Reopen: the blob persisted to disk (durable).
        let store = DiskBlobStore::open(&dir).unwrap();
        let h = Hash::of(b"hello blob");
        assert_eq!(
            store.get(&h).await.unwrap().as_deref(),
            Some(&b"hello blob"[..])
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn disk_store_refuses_a_corrupt_blob() {
        // A file under a hash name whose CONTENTS don't hash to that name = corruption/tamper; get must
        // refuse it, not serve wrong bytes (content-addressed integrity, free).
        let dir = temp_dir("corrupt");
        let mut store = DiskBlobStore::open(&dir).unwrap();
        let h = put_blob(&mut store, b"genuine").await;
        // Tamper: overwrite the blob file with different bytes under the same (now-wrong) hash name.
        let path = sharded_path(&dir, &h);
        std::fs::write(&path, b"tampered").unwrap();
        let err = store.get(&h).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn disk_put_heals_a_corrupt_existing_blob_instead_of_trusting_it() {
        // PR#1010: a blob that rotted/was-tampered on disk still sits under the right name. A put of the
        // genuine bytes must NOT skip on mere existence (which would leave the corruption forever); it
        // must verify the existing file and, finding it bad, rewrite the good bytes.
        let dir = temp_dir("heal");
        let mut store = DiskBlobStore::open(&dir).unwrap();
        let h = put_blob(&mut store, b"genuine").await;
        let path = sharded_path(&dir, &h);
        // Corrupt the on-disk blob (bytes no longer hash to the name).
        std::fs::write(&path, b"rotted!!").unwrap();
        // A get now refuses it (corrupt).
        assert_eq!(
            store.get(&h).await.unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
        // Re-put the genuine bytes: put must heal (rewrite), not trust existence.
        let h2 = put_blob(&mut store, b"genuine").await;
        assert_eq!(h, h2);
        // Now the blob is good again.
        assert_eq!(
            store.get(&h).await.unwrap().as_deref(),
            Some(&b"genuine"[..])
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn disk_put_uses_unique_temp_names_so_concurrent_puts_dont_collide() {
        // PR#1011: two concurrent puts of the SAME content must not share a `{hash}.tmp` path (a torn
        // write would then rename a corrupt file into place). We can't easily race threads deterministically
        // here, but we CAN assert the invariant that makes the race safe: temp names are process-unique.
        // Put twice; a leftover `{hash}.tmp` (the OLD shared name) must never exist, and both succeed.
        let dir = temp_dir("tmp-unique");
        let mut store = DiskBlobStore::open(&dir).unwrap();
        let h1 = put_blob(&mut store, b"same content").await;
        let h2 = put_blob(&mut store, b"same content").await;
        assert_eq!(h1, h2);
        // The old collision-prone temp name must not be present, and only the final blob file remains in the
        // SHARD dir (root/{hh}/{hh}/) — the blob basename is hex[4..], no leftover temp files.
        let hex = h1.to_hex();
        let shard = dir.join(&hex[0..2]).join(&hex[2..4]);
        let shared_tmp = shard.join(format!("{}.tmp", &hex[4..]));
        assert!(
            !shared_tmp.exists(),
            "the shared temp name must not be used"
        );
        let entries: Vec<_> = std::fs::read_dir(&shard)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            entries,
            vec![hex[4..].to_string()],
            "only the final blob remains in its shard — no leftover temp files"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn distinct_content_distinct_keys() {
        let mut store = MemBlobStore::new();
        let a = put_blob(&mut store, b"alpha").await;
        let b = put_blob(&mut store, b"beta").await;
        assert_ne!(a, b);
        assert_eq!(store.len(), 2);
    }
}
