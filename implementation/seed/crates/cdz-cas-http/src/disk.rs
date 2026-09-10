//! An on-disk [`BlobStore`] backend (`DiskBlobStore`) — the persistent backing store for a deployed CAS,
//! so blobs survive a restart (deploy tooling PUTs component bytes once; the store keeps them). The
//! [`CasServer`](crate::CasServer) backend is a swappable trait object, so this drops in behind the same
//! HTTP wire with no protocol change.
//!
//! Each blob is one file under `root`, named by the **content digest** (tag-normalized to `Blob`, rendered
//! base62 — the same string form as the wire, never hex). Keying on the digest (not the tagged hash) makes
//! the store tag-agnostic exactly like [`InMemoryBlobStore`](cdz_platform::InMemoryBlobStore): a component
//! PUT under its `Blob` hash resolves a GET by its `Program` hash over the same bytes.
//!
//! I/O is async (`tokio::fs`) so a fetch never blocks the runtime — the whole point of the async
//! [`BlobStore`] trait (a disk/network backend awaits without stalling the event loop). Writes are atomic
//! (write a unique temp file, then `rename`), so a concurrent reader never observes a partial blob, and a
//! put is idempotent: content-addressed, so re-putting identical bytes is a no-op once the file exists.

use async_trait::async_trait;
use bytes::Bytes;
use cdz_platform::{BlobStore, Hash, HashTag};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// A [`BlobStore`] backed by a directory of content-addressed files. `open` it on a path; the directory is
/// created if absent.
#[derive(Debug)]
pub struct DiskBlobStore {
    root: PathBuf,
    /// A per-process counter making each in-flight temp filename unique, so two concurrent puts of the same
    /// content never write the same temp path (they'd corrupt each other before the atomic rename).
    tmp_counter: AtomicU64,
}

impl DiskBlobStore {
    /// Open (creating if needed) a disk-backed store rooted at `root`.
    ///
    /// # Errors
    /// Propagates an I/O error creating the root directory.
    pub fn open(root: impl Into<PathBuf>) -> std::io::Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        Ok(Self {
            root,
            tmp_counter: AtomicU64::new(0),
        })
    }

    /// The file path for `hash`: keyed on the 32-byte digest (tag ignored), tag-normalized to `Blob` and
    /// rendered base62 — so the same bytes map to one file however their hash is tagged.
    fn path_for(&self, hash: &Hash) -> PathBuf {
        let mut bytes = [0u8; Hash::LEN];
        bytes[0] = HashTag::Blob as u8;
        bytes[1..].copy_from_slice(hash.digest());
        self.root.join(Hash::from_bytes(bytes).to_string())
    }

    /// Write `bytes` to `path` atomically: skip if already present (content-addressed → idempotent), else
    /// write a unique temp file and `rename` it into place (a reader never sees a partial blob).
    async fn write_atomic(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        if tokio::fs::try_exists(path).await.unwrap_or(false) {
            return Ok(());
        }
        let seq = self.tmp_counter.fetch_add(1, Ordering::Relaxed);
        let file_name = path
            .file_name()
            .expect("a blob path always has a base62 file name")
            .to_string_lossy();
        let tmp = path.with_file_name(format!("{file_name}.tmp.{}.{seq}", std::process::id()));
        tokio::fs::write(&tmp, bytes).await?;
        // rename is atomic within a filesystem; if a racing writer already landed the final file, the
        // rename simply replaces it with identical content.
        tokio::fs::rename(&tmp, path).await
    }
}

#[async_trait]
impl BlobStore for DiskBlobStore {
    async fn put(&mut self, bytes: Bytes) -> Hash {
        let hash = Hash::of(HashTag::Blob, &bytes);
        let path = self.path_for(&hash);
        // `put` returns no `Result` (the trait is deterministic — a fallible backend absorbs I/O). On a
        // write error we log and still return the content hash, matching the HTTP client's contract.
        if let Err(err) = self.write_atomic(&path, &bytes).await {
            tracing::error!(error = %err, path = %path.display(), "cas-http disk put failed");
        }
        hash
    }

    async fn get(&self, hash: Hash) -> Option<Bytes> {
        match tokio::fs::read(self.path_for(&hash)).await {
            Ok(bytes) => Some(Bytes::from(bytes)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
            Err(err) => {
                // A non-absence read error (permissions, corruption) can't be surfaced through the trait's
                // `Option`; log it and report a miss (the safe conservative behavior).
                tracing::warn!(error = %err, %hash, "cas-http disk get failed; treating as a miss");
                None
            }
        }
    }

    async fn has(&self, hash: Hash) -> bool {
        tokio::fs::try_exists(self.path_for(&hash))
            .await
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unique scratch directory for a test run.
    fn scratch(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("cdz-cas-disk-{tag}-{}-{nanos}", std::process::id()))
    }

    #[tokio::test]
    async fn round_trips_keys_on_digest_and_reports_absence() {
        let dir = scratch("rt");
        let mut store = DiskBlobStore::open(&dir).expect("open");
        let bytes = Bytes::from_static(b"persisted blob");

        let hash = store.put(bytes.clone()).await;
        assert_eq!(hash, Hash::of(HashTag::Blob, &bytes));
        assert_eq!(store.get(hash).await, Some(bytes.clone()));
        assert!(store.has(hash).await);

        // Digest-keying: the same content fetched by a Program-tagged hash resolves.
        let program = Hash::of(HashTag::Program, b"persisted blob");
        assert_eq!(store.get(program).await, Some(bytes));
        assert!(store.has(program).await);

        // Genuine absence.
        let absent = Hash::of(HashTag::Blob, b"never stored");
        assert_eq!(store.get(absent).await, None);
        assert!(!store.has(absent).await);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn blobs_persist_across_reopen() {
        let dir = scratch("persist");
        let hash = {
            let mut store = DiskBlobStore::open(&dir).expect("open");
            store.put(Bytes::from_static(b"survives restart")).await
        };
        // A fresh store on the same dir still holds the blob (the "survives a restart" property).
        let reopened = DiskBlobStore::open(&dir).expect("reopen");
        assert_eq!(
            reopened.get(hash).await,
            Some(Bytes::from_static(b"survives restart"))
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn put_is_idempotent_by_content() {
        let dir = scratch("idem");
        let mut store = DiskBlobStore::open(&dir).expect("open");
        let h1 = store.put(Bytes::from_static(b"same")).await;
        let h2 = store.put(Bytes::from_static(b"same")).await;
        assert_eq!(h1, h2);
        assert_eq!(store.get(h1).await, Some(Bytes::from_static(b"same")));
        std::fs::remove_dir_all(&dir).ok();
    }
}
