//! DURABLE snapshot of the host's canonical §4c [`NameStore`](cdz_kernel::name_store::NameStore) — the
//! AWS-backends arc, I4a. The deployed daemon holds ONE canonical name directory (the shared §4c name store,
//! see [`AgentHost::with_canonical_store`](crate::host::AgentHost::with_canonical_store)); this module makes
//! that directory SURVIVE a restart: snapshot it to a backend on mutation, restore it on boot.
//!
//! **Why a FIXED-KEY store, NOT the content-addressed [`BlobStore`](cdz_kernel::blob::BlobStore).** A blob
//! store is content-addressed — `put` returns the bytes' Hash and there is no stable "latest" pointer, so it
//! CANNOT serve restore-by-known-location (on boot the daemon doesn't know the current snapshot's hash). The
//! name-store snapshot is the opposite shape: ONE logical object that's OVERWRITTEN each save and read back
//! from a KNOWN location on boot. So it gets its own small host trait ([`NameStoreSnapshotStore`]) whose
//! `save` writes a fixed location and `load` reads it (or `None` if nothing's been saved yet).
//!
//! The kernel already provides the SERIALIZATION —
//! [`NameStore::snapshot_bytes`](cdz_kernel::name_store::NameStore::snapshot_bytes) /
//! [`from_snapshot_bytes`](cdz_kernel::name_store::NameStore::from_snapshot_bytes) (a byte-stable,
//! self-framing blob). This module is only the host-side PERSISTENCE MECHANISM + its wiring; it never edits
//! the kernel.
//!
//! Two impls, mirroring [`s3_blob`](crate::s3_blob)'s layering so the default gate stays hermetic:
//! - [`MemNameStoreSnapshot`] — an in-memory `Option<Vec<u8>>`, always compiled (dev/test; no durability
//!   across a process, but the save/load contract is exercisable with no credentials).
//! - [`S3NameStoreSnapshot`] — behind `live-aws-storage`: a FIXED S3 key (`{prefix}namestore/canonical.snapshot`)
//!   overwritten by `PutObject` and read by `GetObject` (a `NoSuchKey` on load is `Ok(None)`, per the trait
//!   contract). Client construction + credential chain + error mapping mirror [`S3BlobStore`](crate::s3_blob).

/// A FIXED-LOCATION store for the canonical name-store snapshot — the durability seam for the §4c shared
/// name directory (I4a). UNLIKE the content-addressed [`BlobStore`](cdz_kernel::blob::BlobStore), this is a
/// single logical object OVERWRITTEN on each `save` and read back from its known location on boot, so a
/// deployed daemon can restore its name directory without knowing any content hash. `?Send` because the host
/// is single-threaded (the `!Send` registry drives it, §15b) — same discipline as the kernel's `BlobStore`.
#[async_trait::async_trait(?Send)]
pub trait NameStoreSnapshotStore {
    /// Save the latest snapshot to a FIXED location, overwriting the previous one. `Ok(())` once the bytes
    /// are durable (for S3, once `PutObject` acknowledged). The caller passes
    /// [`NameStore::snapshot_bytes`](cdz_kernel::name_store::NameStore::snapshot_bytes).
    async fn save(&mut self, bytes: &[u8]) -> std::io::Result<()>;

    /// Load the latest snapshot, or `None` if none has ever been saved (absence is a normal answer, not an
    /// error — a fresh deployment boots with an empty name store). The caller feeds a returned `Some(bytes)`
    /// to [`NameStore::from_snapshot_bytes`](cdz_kernel::name_store::NameStore::from_snapshot_bytes).
    async fn load(&self) -> std::io::Result<Option<Vec<u8>>>;
}

/// An in-memory [`NameStoreSnapshotStore`] — the always-on dev/test backend. Holds the latest snapshot in an
/// `Option<Vec<u8>>`: `save` overwrites it, `load` clones it out. No durability across a PROCESS (it's just
/// heap), but it exercises the full save/load + restore contract with no credentials, so the mutation-hook +
/// restore-on-boot wiring is testable on the hermetic gate.
#[derive(Debug, Default)]
pub struct MemNameStoreSnapshot {
    latest: Option<Vec<u8>>,
}

impl MemNameStoreSnapshot {
    /// A fresh store with no snapshot saved yet (`load` → `None`).
    pub fn new() -> Self {
        MemNameStoreSnapshot { latest: None }
    }
}

#[async_trait::async_trait(?Send)]
impl NameStoreSnapshotStore for MemNameStoreSnapshot {
    async fn save(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.latest = Some(bytes.to_vec());
        Ok(())
    }

    async fn load(&self) -> std::io::Result<Option<Vec<u8>>> {
        Ok(self.latest.clone())
    }
}

#[cfg(feature = "live-aws-storage")]
pub use s3::S3NameStoreSnapshot;

#[cfg(feature = "live-aws-storage")]
mod s3 {
    use super::NameStoreSnapshotStore;
    use aws_sdk_s3::operation::get_object::GetObjectError;
    use std::io;

    /// The fixed-key S3 [`NameStoreSnapshotStore`] — the AWS-native durability backend for the canonical name
    /// directory (I4a). ONE object per bucket+prefix at [`SNAPSHOT_LEAF`], overwritten each `save`. Client
    /// construction + the SDK default credential chain (env / profile / IMDS via `aws-config`) + the error
    /// mapping mirror [`S3BlobStore`](crate::s3_blob) exactly — no broker, no hardcoded creds. Because the key
    /// is FIXED (not a content hash), there is no sharding / `key_for`: it's a single overwrite target.
    pub struct S3NameStoreSnapshot {
        /// The ambient SDK config; the real S3 `Client` is built LAZILY from it on first I/O (see
        /// [`client`](Self::client)) — NOT at construction — so the store can be built (and its
        /// snapshot-key-format logic tested) WITHOUT an aws-smithy TLS client, which panics in a CA-less
        /// hermetic sandbox. Prod is unaffected (first `save`/`load` builds the client under system roots).
        config: aws_config::SdkConfig,
        client: std::cell::OnceCell<aws_sdk_s3::Client>,
        bucket: String,
        /// Key prefix, normalized to end with `/` when non-empty (empty = bucket root). The snapshot object's
        /// key is `{prefix}{SNAPSHOT_LEAF}`.
        prefix: String,
    }

    /// The fixed key LEAF for the canonical name-store snapshot (under the configured prefix). One object,
    /// overwritten each save — the daemon reads it back from this known location on boot.
    const SNAPSHOT_LEAF: &str = "namestore/canonical.snapshot";

    impl S3NameStoreSnapshot {
        /// Build the store, loading AWS config from the ambient environment (SDK default provider chain), the
        /// SAME construction path as [`S3BlobStore::new`](crate::s3_blob::S3BlobStore::new). `prefix` is
        /// normalized to end with `/` when non-empty (empty = bucket root).
        pub async fn new(bucket: impl Into<String>, prefix: impl Into<String>) -> Self {
            let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
            Self::from_conf(&config, bucket, prefix)
        }

        /// Build from an explicit SDK config (a test/integration harness pointing at a specific region /
        /// endpoint) instead of the ambient default chain.
        pub fn from_conf(
            config: &aws_config::SdkConfig,
            bucket: impl Into<String>,
            prefix: impl Into<String>,
        ) -> Self {
            S3NameStoreSnapshot {
                config: config.clone(),
                client: std::cell::OnceCell::new(),
                bucket: bucket.into(),
                prefix: normalize_prefix(prefix.into()),
            }
        }

        /// The S3 `Client`, built LAZILY from the stored config on first use + cached (mirrors
        /// [`S3BlobStore::client`](crate::s3_blob::S3BlobStore)). Deferring construction to actual I/O keeps
        /// store construction hermetic — the aws-smithy rustls client panics without system CA roots.
        fn client(&self) -> &aws_sdk_s3::Client {
            self.client
                .get_or_init(|| aws_sdk_s3::Client::new(&self.config))
        }

        /// The full S3 object key for the snapshot: `{prefix}{SNAPSHOT_LEAF}` (a fixed key, not a content
        /// hash) — the single object `save` overwrites + `load` reads.
        fn snapshot_key(&self) -> String {
            format!("{}{}", self.prefix, SNAPSHOT_LEAF)
        }
    }

    /// Normalize a key prefix: empty stays empty (bucket root); otherwise ensure a single trailing `/` so the
    /// key derivation is a plain concat (mirrors `s3_blob::normalize_prefix`).
    fn normalize_prefix(mut p: String) -> String {
        if !p.is_empty() && !p.ends_with('/') {
            p.push('/');
        }
        p
    }

    /// Map an S3/SDK error to an `io::Error` (the trait's error type) — same stringly wrap as
    /// `s3_blob::s3_io_err` (an S3 failure is an opaque backend error to the caller, like a disk I/O error).
    fn s3_io_err(context: &str, e: impl std::fmt::Display) -> io::Error {
        io::Error::other(format!("S3 name-store snapshot {context}: {e}"))
    }

    #[async_trait::async_trait(?Send)]
    impl NameStoreSnapshotStore for S3NameStoreSnapshot {
        async fn save(&mut self, bytes: &[u8]) -> io::Result<()> {
            // Unconditional PutObject overwrites the fixed key — the "latest snapshot" is whatever was last
            // written (S3 PutObject is atomic, so there's no torn-write window).
            let key = self.snapshot_key();
            self.client()
                .put_object()
                .bucket(&self.bucket)
                .key(&key)
                .body(bytes.to_vec().into())
                .send()
                .await
                .map_err(|e| s3_io_err("put_object", aws_sdk_s3::error::DisplayErrorContext(&e)))?;
            Ok(())
        }

        async fn load(&self) -> io::Result<Option<Vec<u8>>> {
            let key = self.snapshot_key();
            let resp = match self
                .client()
                .get_object()
                .bucket(&self.bucket)
                .key(&key)
                .send()
                .await
            {
                Ok(resp) => resp,
                // No snapshot saved yet → Ok(None) (a fresh deployment boots empty). S3 signals absence as
                // the typed NoSuchKey service error.
                Err(e) => {
                    return match e.into_service_error() {
                        GetObjectError::NoSuchKey(_) => Ok(None),
                        other => Err(s3_io_err(
                            "get_object",
                            aws_sdk_s3::error::DisplayErrorContext(&other),
                        )),
                    };
                }
            };
            let data = resp
                .body
                .collect()
                .await
                .map_err(|e| s3_io_err("get_object body", e))?
                .into_bytes()
                .to_vec();
            Ok(Some(data))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn prefix_is_normalized_to_a_single_trailing_slash() {
            assert_eq!(normalize_prefix(String::new()), ""); // root stays empty
            assert_eq!(normalize_prefix("names".into()), "names/");
            assert_eq!(normalize_prefix("names/".into()), "names/"); // idempotent
        }

        #[test]
        fn snapshot_key_is_the_fixed_leaf_under_the_prefix() {
            // Construct without touching AWS: from_conf with a minimal config makes no call until an op runs.
            let cfg = aws_config::SdkConfig::builder()
                .behavior_version(aws_config::BehaviorVersion::latest())
                .build();
            let store = S3NameStoreSnapshot::from_conf(&cfg, "my-bucket", "names");
            assert_eq!(store.snapshot_key(), "names/namestore/canonical.snapshot");
            // No prefix → bucket root, still the fixed leaf.
            let rooted = S3NameStoreSnapshot::from_conf(&cfg, "my-bucket", "");
            assert_eq!(rooted.snapshot_key(), "namestore/canonical.snapshot");
        }

        #[test]
        fn from_conf_defers_client_construction() {
            // The lazy-client invariant (mirrors s3_blob/dynamo_log): from_conf builds NO aws client — the
            // OnceCell stays empty until first I/O — so this snapshot store is hermetically constructible.
            let cfg = aws_config::SdkConfig::builder()
                .behavior_version(aws_config::BehaviorVersion::latest())
                .build();
            let store = S3NameStoreSnapshot::from_conf(&cfg, "my-bucket", "names");
            assert!(
                store.client.get().is_none(),
                "from_conf must NOT eagerly build the S3 client (deferred to first I/O)"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_kernel::hash::Hash;
    use cdz_kernel::name_store::{NameStore, SetEntry};

    #[tokio::test]
    async fn mem_snapshot_load_is_none_before_any_save() {
        let store = MemNameStoreSnapshot::new();
        assert_eq!(store.load().await.unwrap(), None, "no snapshot saved yet");
    }

    #[tokio::test]
    async fn mem_snapshot_save_then_load_round_trips_the_bytes() {
        let mut store = MemNameStoreSnapshot::new();
        store.save(b"snapshot-bytes").await.unwrap();
        assert_eq!(
            store.load().await.unwrap().as_deref(),
            Some(&b"snapshot-bytes"[..])
        );
        // A later save OVERWRITES (this is a fixed-location "latest", not an append log).
        store.save(b"newer").await.unwrap();
        assert_eq!(store.load().await.unwrap().as_deref(), Some(&b"newer"[..]));
    }

    #[tokio::test]
    async fn a_real_name_store_round_trips_through_snapshot_save_load_restore() {
        // The end-to-end durability contract: a NameStore with some set-entries → snapshot_bytes → save →
        // load → from_snapshot_bytes reconstructs an identical store (the kernel owns the serialization; this
        // proves the host persistence layer carries those bytes faithfully).
        let (v1, v2) = (Hash::of(b"compiler v1"), Hash::of(b"scratch"));
        let mut original = NameStore::new();
        original
            .set(NameStore::COMPILER_LATEST, SetEntry::unsigned(v1))
            .unwrap();
        original
            .set("session/abc/scratch", SetEntry::unsigned(v2))
            .unwrap();

        let mut store = MemNameStoreSnapshot::new();
        store.save(&original.snapshot_bytes()).await.unwrap();

        let loaded = store.load().await.unwrap().expect("a snapshot was saved");
        let restored = NameStore::from_snapshot_bytes(&loaded).expect("valid snapshot restores");
        assert_eq!(restored.resolve(NameStore::COMPILER_LATEST).unwrap(), v1);
        assert_eq!(restored.resolve("session/abc/scratch").unwrap(), v2);
        assert_eq!(
            restored.to_set_entries(),
            original.to_set_entries(),
            "restore reconstructs an identical store"
        );
    }
}
