//! Recording store decorators — a [`KvStore`] and a [`BlobStore`] that log every call
//! (`design/cadenza-platform.md` §7/§8).
//!
//! The design makes the two stores swappable trait objects, so observation is a decorator, not a
//! change to the kernel: wrap the real backend and, on each call, append a record to the shared
//! [`ObservationLog`] before returning the backend's answer unchanged. The wrapped store behaves
//! exactly as the inner one — same values, same order — so recording never alters a run, only observes
//! it (§9). Each decorator is constructed for one reducer (its [`Origin`]), so a recorded call carries
//! who made it; a run gives each reducer its own recording store over the one shared log, and the log
//! interleaves them in the global [`seq`](crate::Record::seq) order.

use crate::log::{BlobOp, Entry, KvOp, ObservationLog};
use async_trait::async_trait;
use bytes::Bytes;
use cdz_platform::{BlobStore, Hash, KeyRange, KvKeyScan, KvScan, KvStore, Origin};

/// A [`KvStore`] that records every operation to an [`ObservationLog`], then defers to the wrapped
/// store. Generic over the inner backend, so it decorates any `KvStore` — the in-memory one for tests,
/// a persistent one in a fuller harness. It is itself a `KvStore`, so it drops in wherever a store is
/// expected (behind an `Arc`/`Box`, as the platform holds its backends).
pub struct RecordingKvStore<K> {
    inner: K,
    owner: Origin,
    log: ObservationLog,
    now: fn() -> u64,
}

impl<K> RecordingKvStore<K> {
    /// Wrap `inner`, attributing every recorded call to `owner` (the reducer whose store this is),
    /// appending to `log`, and stamping each record with `now` — pass the runtime's clock
    /// (`Runtime::now`) so records carry deterministic simulated time under the bach simulator.
    pub fn new(inner: K, owner: Origin, log: ObservationLog, now: fn() -> u64) -> Self {
        Self {
            inner,
            owner,
            log,
            now,
        }
    }

    /// Recover the wrapped store — e.g. to read the in-memory backend's final state after a run.
    pub fn into_inner(self) -> K {
        self.inner
    }

    fn record(&self, op: KvOp) {
        self.log.record((self.now)(), self.owner, Entry::Kv(op));
    }
}

#[async_trait]
impl<K: KvStore> KvStore for RecordingKvStore<K> {
    async fn get(&self, key: &[u8]) -> Option<Bytes> {
        let value = self.inner.get(key).await;
        self.record(KvOp::Get {
            key: Bytes::copy_from_slice(key),
            hit: value.is_some(),
        });
        value
    }

    async fn put(&mut self, key: Bytes, value: Bytes) {
        // Record the write (O(1) Bytes clones), then apply it. `put` has no outcome to observe, so the
        // order relative to the backend call does not matter; recording first keeps the write and its
        // record adjacent even if the backend were to yield.
        self.record(KvOp::Put {
            key: key.clone(),
            value: value.clone(),
        });
        self.inner.put(key, value).await;
    }

    async fn delete(&mut self, key: &[u8]) -> bool {
        let existed = self.inner.delete(key).await;
        self.record(KvOp::Delete {
            key: Bytes::copy_from_slice(key),
            existed,
        });
        existed
    }

    fn scan(&self, range: KeyRange) -> KvScan<'_> {
        self.record(KvOp::Scan {
            lower: range.0.clone(),
            upper: range.1.clone(),
            keys_only: false,
        });
        self.inner.scan(range)
    }

    fn scan_keys(&self, range: KeyRange) -> KvKeyScan<'_> {
        self.record(KvOp::Scan {
            lower: range.0.clone(),
            upper: range.1.clone(),
            keys_only: true,
        });
        self.inner.scan_keys(range)
    }
}

/// A [`BlobStore`] that records every operation to an [`ObservationLog`], then defers to the wrapped
/// store. Like [`RecordingKvStore`], it is itself a `BlobStore` and observes without altering the
/// backend's answers.
pub struct RecordingBlobStore<B> {
    inner: B,
    owner: Origin,
    log: ObservationLog,
    now: fn() -> u64,
}

impl<B> RecordingBlobStore<B> {
    /// Wrap `inner`, attributing every recorded call to `owner`, appending to `log`, and stamping each
    /// record with `now` (the runtime's clock).
    pub fn new(inner: B, owner: Origin, log: ObservationLog, now: fn() -> u64) -> Self {
        Self {
            inner,
            owner,
            log,
            now,
        }
    }

    /// Recover the wrapped store.
    pub fn into_inner(self) -> B {
        self.inner
    }

    fn record(&self, op: BlobOp) {
        self.log.record((self.now)(), self.owner, Entry::Blob(op));
    }
}

#[async_trait]
impl<B: BlobStore> BlobStore for RecordingBlobStore<B> {
    async fn put(&mut self, bytes: Bytes) -> Hash {
        // The stored bytes are addressed by the hash, so the record keeps the hash and the byte length,
        // not the bytes again. Capture the length before the bytes move into the backend.
        let len = bytes.len();
        let hash = self.inner.put(bytes).await;
        self.record(BlobOp::Put { hash, len });
        hash
    }

    async fn get(&self, hash: Hash) -> Option<Bytes> {
        let bytes = self.inner.get(hash).await;
        self.record(BlobOp::Get {
            hash,
            hit: bytes.is_some(),
        });
        bytes
    }

    async fn has(&self, hash: Hash) -> bool {
        let present = self.inner.has(hash).await;
        self.record(BlobOp::Has { hash, present });
        present
    }
}
