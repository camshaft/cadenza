//! GAP-4 D3 checkpoint SNAPSHOT persistence — the host half of log-prune-to-checkpoint.
//!
//! The kernel provides the checkpoint MECHANISM ([`Session::checkpoint_frame`](cdz_kernel::kernel::Session::checkpoint_frame)
//! / [`checkpoint_subsumed_prefix`](cdz_kernel::kernel::Session::checkpoint_subsumed_prefix) /
//! [`recover_from_checkpoint`](cdz_kernel::kernel::Session::recover_from_checkpoint)); the durable checkpoint
//! frame carries only the KV *root hash* (`kv_root`), not the KV content. So the host must PERSIST the KV
//! snapshot bytes when it checkpoints and RELOAD them on recovery to seed `recover_from_checkpoint`.
//!
//! Both are one call over the SAME content-addressed store the D1 body-offload uses (an
//! [`OffloadSource`](crate::factory::OffloadSource)-materialized [`BlobStore`]): a KV's canonical
//! [`Kv::encode`] is content-addressed by [`Kv::root_hash`] (which IS `Hash::of(encode())`, kv.rs), and
//! that hash is EXACTLY the checkpoint descriptor's `kv_root`. So the snapshot is stored under — and reloaded
//! by — the very hash the descriptor names: no side index, no drift. A content-verifying store's `get`
//! self-checks the bytes back to `kv_root`, `Kv::decode` round-trips them, and `recover_from_checkpoint`
//! re-verifies `seed_kv.root_hash() == descriptor.kv_root` (a wrong snapshot is refused). This is the D3-1
//! primitive the per-backend checkpoint-write/recovery paths (D3-2 File, D3-3 Dynamo) build on.

use cdz_kernel::blob::BlobStore;
use cdz_kernel::hash::Hash;
use cdz_kernel::kv::Kv;

/// Persist `kv`'s canonical snapshot to `blob`, content-addressed by its root hash — which IS the checkpoint
/// descriptor's `kv_root`. Returns that root so the caller can assert it matches the checkpoint frame it is
/// about to append (the crash-safe protocol persists the snapshot BEFORE appending the frame). Idempotent by
/// content-addressing: re-persisting an identical KV is a no-op write.
pub async fn persist_kv_snapshot(blob: &mut dyn BlobStore, kv: &Kv) -> Result<Hash, String> {
    let root = kv.root_hash();
    blob.put(root, bytes::Bytes::from(kv.encode()))
        .await
        .map_err(|e| format!("checkpoint: could not persist kv snapshot {root}: {e}"))?;
    Ok(root)
}

/// Reload the KV snapshot the descriptor's `kv_root` names from `blob`, decoding it back to a [`Kv`] ready to
/// seed [`recover_from_checkpoint`](cdz_kernel::kernel::Session::recover_from_checkpoint). `Ok(None)` = the
/// snapshot is ABSENT (the checkpoint names a snapshot the store doesn't hold — the caller falls back to
/// full-log replay, §4, rather than failing). `Err` = a store I/O error, or a present-but-corrupt snapshot
/// that doesn't decode (fail cleanly, never a panic — §17; the caller likewise falls back to replay/alarms).
pub async fn load_kv_snapshot(blob: &dyn BlobStore, kv_root: Hash) -> Result<Option<Kv>, String> {
    let Some(bytes) = blob
        .get(&kv_root)
        .await
        .map_err(|e| format!("checkpoint: kv snapshot {kv_root} get failed: {e}"))?
    else {
        return Ok(None);
    };
    let kv = Kv::decode(&bytes)
        .map_err(|e| format!("checkpoint: kv snapshot {kv_root} did not decode: {e:?}"))?;
    Ok(Some(kv))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_kernel::blob::MemBlobStore;

    fn sample_kv() -> Kv {
        let mut kv = Kv::new();
        kv.put(b"alpha".to_vec(), b"one".to_vec());
        kv.put(b"beta".to_vec(), b"two".to_vec());
        kv.put(b"inbox/0001".to_vec(), b"payload".to_vec());
        kv
    }

    #[tokio::test]
    async fn persist_then_load_round_trips_the_kv_snapshot_under_its_root() {
        // The D3-1 core contract: a KV snapshot persists under its root hash (== a checkpoint descriptor's
        // kv_root) and reloads byte-identical — the seed_kv recover_from_checkpoint needs.
        let mut blob = MemBlobStore::new();
        let kv = sample_kv();
        let root = persist_kv_snapshot(&mut blob, &kv).await.expect("persist");
        assert_eq!(
            root,
            kv.root_hash(),
            "the snapshot is keyed by the KV root hash — exactly what a checkpoint descriptor names"
        );
        let reloaded = load_kv_snapshot(&blob, root)
            .await
            .expect("load ok")
            .expect("the snapshot we just persisted is present");
        assert_eq!(
            reloaded.root_hash(),
            root,
            "the reloaded snapshot carries the descriptor's kv_root (recover_from_checkpoint re-verifies this)"
        );
        assert_eq!(reloaded.get(b"alpha").as_deref(), Some(&b"one"[..]));
        assert_eq!(reloaded.get(b"beta").as_deref(), Some(&b"two"[..]));
        assert_eq!(
            reloaded.get(b"inbox/0001").as_deref(),
            Some(&b"payload"[..])
        );
        assert_eq!(reloaded.len(), kv.len(), "no entries lost/gained");
    }

    #[tokio::test]
    async fn load_of_an_absent_snapshot_is_none_not_err() {
        // A checkpoint whose snapshot bytes aren't in the store -> Ok(None): a get-miss is a normal answer
        // (fall back to full-log replay), NOT corruption.
        let blob = MemBlobStore::new();
        let never_persisted = sample_kv().root_hash();
        let got = load_kv_snapshot(&blob, never_persisted)
            .await
            .expect("get of an absent snapshot is Ok");
        assert!(
            got.is_none(),
            "an absent snapshot loads as None, not an error"
        );
    }

    #[tokio::test]
    async fn load_of_a_corrupt_snapshot_is_a_clean_err_not_a_panic() {
        // Bytes present under a key but NOT a valid KV encoding -> clean Err (§17 total decode), never a
        // panic. MemBlobStore does not self-verify, so we can store non-KV bytes content-addressed by their
        // own hash and drive the decode-failure path.
        let mut blob = MemBlobStore::new();
        let garbage = bytes::Bytes::from_static(b"this is not a valid kv encoding");
        let key = Hash::of(&garbage);
        blob.put(key, garbage).await.unwrap();
        let err = load_kv_snapshot(&blob, key)
            .await
            .expect_err("a present-but-corrupt snapshot must be an Err, not a panic");
        assert!(err.contains("did not decode"), "{err}");
    }
}
