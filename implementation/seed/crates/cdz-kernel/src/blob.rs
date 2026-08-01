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
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A content-addressable blob store: put bytes (keyed by their content hash), get them back by hash.
/// Backends are swappable (in-memory, disk, later network). Errors are backend I/O; a missing blob is
/// `Ok(None)`, not an error (absence is a normal answer — the caller decides if it's fatal).
pub trait BlobStore {
    /// Store `bytes` and return their content hash (the key). Idempotent: storing bytes already
    /// present is a no-op that returns the same hash (content-addressed → the key can't collide with
    /// different content).
    fn put(&mut self, bytes: &[u8]) -> std::io::Result<Hash>;

    /// Fetch the bytes for `hash`, or `None` if absent. A backend that can verify integrity SHOULD
    /// (re-hash the bytes, refuse a mismatch) — content-addressing makes tamper-detection free.
    fn get(&self, hash: &Hash) -> std::io::Result<Option<Vec<u8>>>;

    /// Is a blob present without fetching it? Cheap existence check (a disk backend stats the file).
    fn has(&self, hash: &Hash) -> std::io::Result<bool> {
        Ok(self.get(hash)?.is_some())
    }
}

/// In-memory blob store — for tests and single-process use where durability isn't needed.
#[derive(Default)]
pub struct MemBlobStore {
    blobs: HashMap<Hash, Vec<u8>>,
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

impl BlobStore for MemBlobStore {
    fn put(&mut self, bytes: &[u8]) -> std::io::Result<Hash> {
        let hash = Hash::of(bytes);
        // Idempotent: only insert if absent (content-addressed → an existing key holds identical bytes).
        self.blobs.entry(hash).or_insert_with(|| bytes.to_vec());
        Ok(hash)
    }

    fn get(&self, hash: &Hash) -> std::io::Result<Option<Vec<u8>>> {
        Ok(self.blobs.get(hash).cloned())
    }

    fn has(&self, hash: &Hash) -> std::io::Result<bool> {
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

    fn path_for(&self, hash: &Hash) -> PathBuf {
        self.root.join(hash.to_hex())
    }
}

impl BlobStore for DiskBlobStore {
    fn put(&mut self, bytes: &[u8]) -> std::io::Result<Hash> {
        let hash = Hash::of(bytes);
        let path = self.path_for(&hash);
        // Idempotent: if the file already exists, the content is identical (content-addressed) — skip
        // the write. Write to a temp file + rename so a crash mid-write never leaves a partial blob
        // under the final (hash) name (a torn blob would then fail the get-time hash check anyway, but
        // atomic rename avoids the corrupt-file state entirely).
        if path.exists() {
            return Ok(hash);
        }
        let tmp = self.root.join(format!("{}.tmp", hash.to_hex()));
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &path)?;
        Ok(hash)
    }

    fn get(&self, hash: &Hash) -> std::io::Result<Option<Vec<u8>>> {
        let path = self.path_for(hash);
        match std::fs::read(&path) {
            Ok(bytes) => {
                // Self-verify (integrity is free with content-addressing): the file's bytes MUST hash
                // to the requested key, or it's corrupt/tampered — refuse to serve it.
                if Hash::of(&bytes) == *hash {
                    Ok(Some(bytes))
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

    fn has(&self, hash: &Hash) -> std::io::Result<bool> {
        Ok(self.path_for(hash).exists())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("cdz-kernel-blob-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&p);
        p
    }

    fn round_trip<S: BlobStore>(store: &mut S) {
        let h = store.put(b"hello blob").unwrap();
        // The key is the content hash.
        assert_eq!(h, Hash::of(b"hello blob"));
        assert_eq!(store.get(&h).unwrap().as_deref(), Some(&b"hello blob"[..]));
        assert!(store.has(&h).unwrap());
        // Absent blob → None (not an error).
        assert_eq!(store.get(&Hash::of(b"never stored")).unwrap(), None);
        assert!(!store.has(&Hash::of(b"never stored")).unwrap());
        // Idempotent put: same bytes → same hash, no duplicate.
        let h2 = store.put(b"hello blob").unwrap();
        assert_eq!(h, h2);
    }

    #[test]
    fn mem_store_round_trips() {
        let mut store = MemBlobStore::new();
        round_trip(&mut store);
        // idempotent put didn't grow the store beyond the distinct blobs stored.
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn disk_store_round_trips_and_persists() {
        let dir = temp_dir("roundtrip");
        {
            let mut store = DiskBlobStore::open(&dir).unwrap();
            round_trip(&mut store);
        }
        // Reopen: the blob persisted to disk (durable).
        let store = DiskBlobStore::open(&dir).unwrap();
        let h = Hash::of(b"hello blob");
        assert_eq!(store.get(&h).unwrap().as_deref(), Some(&b"hello blob"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn disk_store_refuses_a_corrupt_blob() {
        // A file under a hash name whose CONTENTS don't hash to that name = corruption/tamper; get must
        // refuse it, not serve wrong bytes (content-addressed integrity, free).
        let dir = temp_dir("corrupt");
        let mut store = DiskBlobStore::open(&dir).unwrap();
        let h = store.put(b"genuine").unwrap();
        // Tamper: overwrite the blob file with different bytes under the same (now-wrong) hash name.
        let path = dir.join(h.to_hex());
        std::fs::write(&path, b"tampered").unwrap();
        let err = store.get(&h).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn distinct_content_distinct_keys() {
        let mut store = MemBlobStore::new();
        let a = store.put(b"alpha").unwrap();
        let b = store.put(b"beta").unwrap();
        assert_ne!(a, b);
        assert_eq!(store.len(), 2);
    }
}
