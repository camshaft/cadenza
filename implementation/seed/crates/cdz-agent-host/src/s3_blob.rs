//! S3-backed [`BlobStore`] — the AWS-native content-addressable store (operator: "content-addressed blobs are
//! the obvious thing to put in S3"). This is the first AWS-native host storage backend (I1 of the AWS-backends
//! arc); it drops in behind the SAME `cdz_kernel::blob::BlobStore` trait the in-memory / on-disk backends
//! satisfy, so a consumer holds `Box<dyn BlobStore>` and never sees an AWS type — the daemon just selects this
//! backend by config (`BlobConfig::S3`) when the `live-aws-storage` feature is compiled.
//!
//! **Feature-gated (`live-aws-storage`).** The whole module (+ the aws-sdk-s3 tree) is behind that feature so
//! the DEFAULT build pulls no AWS SDK and tests use `MemBlobStore`/`DiskBlobStore` with no credentials — the
//! hermetic-gate discipline, exactly like `live-net` gates the network EFFECT executors. Credentials come from
//! the SDK default provider chain (env / profile / IMDS) via `aws-config`, the same contract as the Bedrock
//! transport — no broker, no hardcoding.
//!
//! **Content-addressed + self-verifying (same discipline as `DiskBlobStore`).** The object key is the blob's
//! content-hash hex, SHARDED into a fanned key `{prefix}{hh}/{hh}/{rest}` (so the keyspace isn't a single flat
//! namespace — operator review; mirrors `DiskBlobStore`'s on-disk layout), so:
//! - `put` is idempotent — the same bytes always map to the same key; re-putting is a harmless overwrite of
//!   byte-identical content (content-addressing makes the key collision-free for distinct content).
//! - `get` re-hashes the fetched object and REFUSES to serve bytes that don't hash to the requested key (a
//!   corrupted/tampered object is caught, not returned as valid) — integrity is free with content-addressing.
//! - a missing object is `Ok(None)`, not an error (absence is a normal answer, per the trait contract).

use aws_sdk_s3::operation::get_object::GetObjectError;
use aws_sdk_s3::operation::head_object::HeadObjectError;
use cdz_kernel::blob::BlobStore;
use cdz_kernel::hash::Hash;
use std::io;

/// An S3-backed content-addressable blob store. Holds the S3 client + the target bucket + an optional key
/// prefix (so multiple logical stores can share a bucket under distinct prefixes, e.g. `reducers/`). Cheap to
/// clone conceptually (the client is `Arc`-backed), though the trait takes `&mut self` only for `put`.
pub struct S3BlobStore {
    client: aws_sdk_s3::Client,
    bucket: String,
    /// Key prefix prepended to every blob's content-hash hex (empty = bucket root). A non-empty prefix is
    /// stored already-normalized to end with `/` so `key_for` is a plain concat.
    prefix: String,
}

impl S3BlobStore {
    /// Build the store, loading AWS config from the ambient environment (SDK default provider chain + region
    /// from env), the same construction path as [`BedrockModelTransport::new`](crate::BedrockModelTransport).
    /// Async because the default chain may probe the environment (e.g. IMDS). A missing region / unresolvable
    /// credentials surface later, per-operation, as an I/O error — construction just wires the client.
    /// `prefix` is normalized to end with `/` when non-empty (empty = bucket root).
    pub async fn new(bucket: impl Into<String>, prefix: impl Into<String>) -> Self {
        let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        Self::from_conf(&config, bucket, prefix)
    }

    /// Build from an explicit SDK config (a caller that already loaded one, or a test/integration harness
    /// pointing at a specific region / endpoint) instead of the ambient default chain.
    pub fn from_conf(
        config: &aws_config::SdkConfig,
        bucket: impl Into<String>,
        prefix: impl Into<String>,
    ) -> Self {
        S3BlobStore {
            client: aws_sdk_s3::Client::new(config),
            bucket: bucket.into(),
            prefix: normalize_prefix(prefix.into()),
        }
    }

    /// The S3 object key for a blob: `{prefix}{hh}/{hh}/{rest}` — the content-hash hex SHARDED into a fanned
    /// key so the keyspace isn't a single flat namespace (operator review r3735105548 on #2548). The first two
    /// hex byte-pairs become nested key segments, the rest is the leaf (`{hex[0..2]}/{hex[2..4]}/{hex[4..]}`),
    /// mirroring the on-disk `DiskBlobStore` layout so both backends shard identically. blake3 hex is 64 chars,
    /// so the two 2-char shard segments + a 60-char leaf always exist. Sharding is INTERNAL to key derivation
    /// (content-addressed) — the `BlobStore` trait + callers are unaffected; `get`/`has`/`put` all route
    /// through `key_for`, so a blob written under the sharded key is read back under the same one.
    fn key_for(&self, hash: &Hash) -> String {
        let hex = hash.to_hex();
        // blake3 hex = 64 chars (32 bytes); the byte-pair slices are always in range.
        format!("{}{}/{}/{}", self.prefix, &hex[0..2], &hex[2..4], &hex[4..])
    }
}

/// Normalize a key prefix: empty stays empty (bucket root); otherwise ensure a single trailing `/` so
/// `key_for` is a plain concat and callers needn't remember the slash.
fn normalize_prefix(mut p: String) -> String {
    if !p.is_empty() && !p.ends_with('/') {
        p.push('/');
    }
    p
}

/// Map an S3/SDK error to an `io::Error` (the trait's error type). Stringly wrapped — the blob trait's surface
/// is `io::Result`, and an S3 failure is an opaque backend error to the caller (like a disk I/O error).
fn s3_io_err(context: &str, e: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!("S3 blob store {context}: {e}"))
}

#[async_trait::async_trait(?Send)]
impl BlobStore for S3BlobStore {
    async fn put(&mut self, hash: Hash, bytes: bytes::Bytes) -> io::Result<()> {
        // The content hash is SUPPLIED by the caller (computed once at the top, not re-hashed per tier). The
        // key IS that content hash.
        let key = self.key_for(&hash);
        // Idempotent by content-addressing: PutObject overwrites, and the key IS the content hash, so a
        // re-put writes byte-identical content — never a mutation. (We don't HEAD-then-skip: an unconditional
        // put is one round-trip + self-heals a corrupted object, mirroring DiskBlobStore's heal-on-put intent
        // without the extra GET; S3 PutObject is atomic, so there's no torn-write window to guard.)
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&key)
            .body(bytes.into())
            .send()
            .await
            .map_err(|e| s3_io_err("put_object", aws_sdk_s3::error::DisplayErrorContext(&e)))?;
        Ok(())
    }

    async fn get(&self, hash: &Hash) -> io::Result<Option<bytes::Bytes>> {
        let key = self.key_for(hash);
        let resp = match self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
        {
            Ok(resp) => resp,
            // A missing object is Ok(None), not an error (trait contract: absence is a normal answer). S3
            // signals it as the typed NoSuchKey service error.
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
        // Collect the streamed body into ref-counted `Bytes` (no extra copy — `into_bytes` already yields
        // `bytes::Bytes`).
        let data = resp
            .body
            .collect()
            .await
            .map_err(|e| s3_io_err("get_object body", e))?
            .into_bytes();
        // Self-verify (integrity is free with content-addressing): the object's bytes MUST hash to the
        // requested key, or it's corrupt/tampered — refuse to serve it (same discipline as DiskBlobStore).
        if Hash::of(&data) == *hash {
            Ok(Some(data))
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("S3 blob {hash} failed content-hash verification (corrupt/tampered)"),
            ))
        }
    }

    async fn has(&self, hash: &Hash) -> io::Result<bool> {
        // A real existence probe (HEAD), cheaper than a full GET — override the trait's get-based default.
        let key = self.key_for(hash);
        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(e) => match e.into_service_error() {
                HeadObjectError::NotFound(_) => Ok(false),
                other => Err(s3_io_err(
                    "head_object",
                    aws_sdk_s3::error::DisplayErrorContext(&other),
                )),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_is_normalized_to_a_single_trailing_slash() {
        assert_eq!(normalize_prefix(String::new()), ""); // root stays empty
        assert_eq!(normalize_prefix("reducers".into()), "reducers/");
        assert_eq!(normalize_prefix("reducers/".into()), "reducers/"); // idempotent
    }

    #[test]
    fn key_for_shards_the_content_hash_under_the_prefix() {
        // Construct without touching AWS: from_conf with a minimal config never makes a call until an op runs.
        let cfg = aws_config::SdkConfig::builder()
            .behavior_version(aws_config::BehaviorVersion::latest())
            .build();
        let store = S3BlobStore::from_conf(&cfg, "my-bucket", "reducers");
        let h = Hash::of(b"hello blob");
        let hex = h.to_hex();
        // Sharded: {prefix}{hex[0..2]}/{hex[2..4]}/{hex[4..]} — a fanned key, not a flat prefix (operator
        // review r3735105548). Mirrors DiskBlobStore's on-disk layout.
        assert_eq!(
            store.key_for(&h),
            format!("reducers/{}/{}/{}", &hex[0..2], &hex[2..4], &hex[4..])
        );
        // The shard segments are the first two hex byte-pairs; the leaf is the remaining 60 chars.
        assert_eq!(
            store.key_for(&h),
            format!("reducers/{}", shard_suffix(&hex))
        );

        // No prefix → bucket root, still sharded.
        let rooted = S3BlobStore::from_conf(&cfg, "my-bucket", "");
        assert_eq!(rooted.key_for(&h), shard_suffix(&hex));
    }

    /// The sharded suffix `{hex[0..2]}/{hex[2..4]}/{hex[4..]}` — test helper mirroring `key_for`'s fan-out.
    #[cfg(test)]
    fn shard_suffix(hex: &str) -> String {
        format!("{}/{}/{}", &hex[0..2], &hex[2..4], &hex[4..])
    }
}
