//! An S3-backed [`BlobStore`] (`S3BlobStore`) — the durable floor of a
//! [`TieredBlobStore`](crate::TieredBlobStore). Behind the `s3` feature (off by default).
//!
//! Content-addressed objects: a blob is stored under `{prefix}{base62-hash}` (the same base62 `Hash`
//! string used on the wire and on disk, tag-normalized to `Blob`), so a component put under its `Blob`
//! hash resolves a GET by its `Program` hash over the same bytes (§8, tag-agnostic).
//!
//! [`connect`](S3BlobStore::connect) uses the AWS SDK's default configuration — which honors the machine's
//! **configured profiles**: `AWS_PROFILE` / `~/.aws/config` / `~/.aws/credentials`, environment variables,
//! SSO, and the EC2/ECS instance metadata chain — so it "just works" against whatever profile the host is
//! set up with. [`with_client`](S3BlobStore::with_client) takes a preconfigured client for custom setups
//! (an endpoint override for localstack, an explicit region/credentials).

use async_trait::async_trait;
use aws_sdk_s3::Client;
use aws_sdk_s3::primitives::ByteStream;
use bytes::Bytes;
use cdz_platform::{BlobStore, BlobStoreError, Hash, HashTag};
use std::sync::Once;

/// Install the aws-lc-rs rustls [`CryptoProvider`](rustls::crypto::CryptoProvider) as the process default,
/// once. aws-smithy-http-client's default rustls TLS resolves its provider from the process default and
/// PANICS if none is installed, so any code path that builds an S3 [`Client`] must install it first.
/// Idempotent — a prior install (e.g. by the HTTP client) is fine.
fn ensure_aws_lc_provider() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

/// An S3 (or S3-compatible) durable content-addressed store.
#[derive(Clone)]
pub struct S3BlobStore {
    client: Client,
    bucket: String,
    /// A key prefix (empty, or ending in the separator the caller wants, e.g. `blobs/`). Normalized to end
    /// with `/` when non-empty.
    prefix: String,
}

impl S3BlobStore {
    /// Connect to `bucket` using the AWS SDK's DEFAULT configuration (honors the machine's configured
    /// profiles: `AWS_PROFILE`/`~/.aws/*`, env, SSO, IMDS). `prefix` is prepended to every object key
    /// (pass `""` for none).
    pub async fn connect(bucket: impl Into<String>, prefix: impl Into<String>) -> Self {
        // Install the aws-lc-rs crypto provider before any client (incl. the credential chain's HTTP calls)
        // is built, else aws-smithy-http-client's rustls TLS panics.
        ensure_aws_lc_provider();
        let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        Self::with_client(Client::new(&config), bucket, prefix)
    }

    /// Connect from config parts: the default credential chain (honoring configured profiles), with an
    /// optional `region` and `endpoint` override (the latter for an S3-compatible endpoint / localstack).
    pub async fn from_config(
        bucket: &str,
        prefix: &str,
        region: Option<&str>,
        endpoint: Option<&str>,
    ) -> Self {
        ensure_aws_lc_provider();
        let mut loader = aws_config::defaults(aws_config::BehaviorVersion::latest());
        if let Some(region) = region {
            loader = loader.region(aws_config::Region::new(region.to_string()));
        }
        if let Some(endpoint) = endpoint {
            loader = loader.endpoint_url(endpoint.to_string());
        }
        let config = loader.load().await;
        Self::with_client(Client::new(&config), bucket, prefix)
    }

    /// Build from a preconfigured [`Client`] — for a custom endpoint (localstack), region, or credentials.
    #[must_use]
    pub fn with_client(
        client: Client,
        bucket: impl Into<String>,
        prefix: impl Into<String>,
    ) -> Self {
        let mut prefix = prefix.into();
        if !prefix.is_empty() && !prefix.ends_with('/') {
            prefix.push('/');
        }
        Self {
            client,
            bucket: bucket.into(),
            prefix,
        }
    }

    /// The object key for `hash`: `{prefix}{base62}`, keyed on the digest (tag-normalized to `Blob`) so the
    /// same bytes map to one object however their hash is tagged.
    fn key(&self, hash: &Hash) -> String {
        let mut bytes = [0u8; Hash::LEN];
        bytes[0] = HashTag::Blob as u8;
        bytes[1..].copy_from_slice(hash.digest());
        format!("{}{}", self.prefix, Hash::from_bytes(bytes))
    }
}

#[async_trait]
impl BlobStore for S3BlobStore {
    async fn put(&self, bytes: Bytes) -> Result<Hash, BlobStoreError> {
        let hash = Hash::of(HashTag::Blob, &bytes);
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(self.key(&hash))
            .body(ByteStream::from(bytes))
            .send()
            .await
            .map_err(|err| BlobStoreError::Io(format!("s3 put_object: {err}")))?;
        Ok(hash)
    }

    async fn get(&self, hash: Hash) -> Result<Option<Bytes>, BlobStoreError> {
        match self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(self.key(&hash))
            .send()
            .await
        {
            Ok(resp) => {
                let data = resp
                    .body
                    .collect()
                    .await
                    .map_err(|err| BlobStoreError::Io(format!("s3 read body: {err}")))?;
                Ok(Some(data.into_bytes()))
            }
            // A missing object is a genuine miss; anything else is a real error.
            Err(err) if err.as_service_error().is_some_and(|e| e.is_no_such_key()) => Ok(None),
            Err(err) => Err(BlobStoreError::Io(format!("s3 get_object: {err}"))),
        }
    }

    async fn has(&self, hash: Hash) -> Result<bool, BlobStoreError> {
        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(self.key(&hash))
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(err) if err.as_service_error().is_some_and(|e| e.is_not_found()) => Ok(false),
            Err(err) => Err(BlobStoreError::Io(format!("s3 head_object: {err}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Build an S3BlobStore without connecting, to exercise the pure key derivation. A dummy client is built
    // from a minimal config (no network is touched — we only call `key`).
    fn store(prefix: &str) -> S3BlobStore {
        // Building a client constructs its rustls TLS connector — install the provider first.
        ensure_aws_lc_provider();
        let config = aws_sdk_s3::Config::builder()
            .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
            .region(aws_sdk_s3::config::Region::new("us-east-1"))
            .build();
        S3BlobStore::with_client(Client::from_conf(config), "test-bucket", prefix)
    }

    #[test]
    fn key_is_prefix_plus_base62_hash_keyed_on_digest() {
        let s = store("blobs/");
        let bytes = Bytes::from_static(b"hello");
        let blob = Hash::of(HashTag::Blob, &bytes);
        let program = Hash::of(HashTag::Program, &bytes); // same digest, different tag
        // Both tags over the same bytes map to the SAME key (tag-normalized to Blob).
        assert_eq!(s.key(&blob), s.key(&program));
        assert!(s.key(&blob).starts_with("blobs/"));
        // The key is the prefix + the base62 of the Blob-tagged hash.
        assert_eq!(s.key(&blob), format!("blobs/{blob}"));
    }

    #[test]
    fn prefix_is_normalized_to_end_with_a_slash() {
        // A non-empty prefix without a trailing slash gets one; an empty prefix stays empty.
        let with = store("myprefix");
        let bytes = Bytes::from_static(b"x");
        let h = Hash::of(HashTag::Blob, &bytes);
        assert_eq!(with.key(&h), format!("myprefix/{h}"));
        let none = store("");
        assert_eq!(none.key(&h), format!("{h}"));
    }
}
