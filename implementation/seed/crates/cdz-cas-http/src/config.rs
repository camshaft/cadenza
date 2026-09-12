//! The `cdz-cas-http` server configuration — a **binary-AST document** (operator directive: binary-AST is
//! THE data-exchange format). The caller authors the config in any surface syntax (ML/sexpr/JSON/TOML) and
//! converts it to binary-AST (`cdz convert --to bin`); the server reads the bytes, decodes them into a
//! [`ServerConfig`], and assembles the tiered [`BlobStore`] stack from it — so there is no sprawl of CLI
//! flags, and the config's schema is open (whatever the caller's syntax expresses, as long as the record
//! fields below are present).
//!
//! ## The config record
//! A Cadenza record (optionally root-ascribed `(: <record> <Type>)`) with these fields (all optional except
//! where a nested record is present):
//! - `listen` (string) — the `host:port` to bind (default `127.0.0.1:8080`).
//! - `read-credential` / `write-credential` (string) — Bearer credentials; absent ⇒ open reads / writes
//!   disabled.
//! - `max-body-bytes` (int) — the per-write body ceiling.
//! - `mem-cache-bytes` (int) — if present, a byte-budgeted in-memory S3-FIFO cache tier.
//! - `disk-cache` (record `{ dir, bytes }`) — if present, a size-capped on-disk cache tier.
//! - `s3` (record `{ bucket, prefix?, region?, endpoint? }`) — if present (and the binary was built
//!   `--features s3`), the durable S3 floor.
//!
//! The tier stack is `[mem?, disk?, s3?]` shallowest→deepest; with nothing configured it's a single
//! in-memory store.

use crate::{BoundedDiskCache, MemoryCache, TieredBlobStore};
use cdz_platform::{BlobStore, InMemoryBlobStore};
use serde::Deserialize;
use std::sync::Arc;

/// A decoded server configuration.
///
/// Decoded from a binary-AST record via `serde` (through [`cadenza_ast_serde`]). `rename_all =
/// "kebab-case"` maps the fields to the record's kebab keys (`read-credential`, `max-body-bytes`, …);
/// `default` (the struct is `Default`) fills every ABSENT field with its default (`None` for the
/// options) — matching the old reader's "all fields optional" behavior.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ServerConfig {
    pub listen: Option<String>,
    pub read_credential: Option<String>,
    pub write_credential: Option<String>,
    pub max_body_bytes: Option<usize>,
    pub mem_cache_bytes: Option<usize>,
    pub disk_cache: Option<DiskCacheConfig>,
    pub s3: Option<S3Config>,
}

/// The on-disk cache tier config. `dir` + `bytes` are REQUIRED when a `disk-cache` record is present
/// (a missing one is a decode error — the serde analogue of the old `MissingField`).
#[derive(Debug, Clone, Deserialize)]
pub struct DiskCacheConfig {
    pub dir: String,
    pub bytes: usize,
}

/// The S3 durable-floor config. `bucket` is REQUIRED; `prefix` defaults to empty; `region`/`endpoint`
/// default to `None` when absent (matching the old reader).
#[derive(Debug, Clone, Deserialize)]
pub struct S3Config {
    pub bucket: String,
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
}

/// Why a config could not be decoded or its store assembled.
#[derive(Debug)]
pub enum ConfigError {
    /// The bytes could not be decoded into a `ServerConfig` — not a binary-AST document, the root is
    /// not a record, a required field is missing or ill-typed, etc. Carries the serde decoder's reason
    /// (it subsumes the old `Decode` / `NotARecord` / `MissingField` cases with a richer message).
    Decode(String),
    /// A configured tier failed to initialize (e.g. the disk cache directory).
    Tier(String),
    /// The config asked for an S3 tier but the binary was built without `--features s3`.
    S3Unsupported,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode(msg) => write!(f, "config decode failed: {msg}"),
            Self::Tier(msg) => write!(f, "config tier init failed: {msg}"),
            Self::S3Unsupported => write!(
                f,
                "config specifies an `s3` tier but this binary was built without `--features s3`"
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

impl ServerConfig {
    /// Decode a binary-AST config document.
    ///
    /// # Errors
    /// [`ConfigError::Decode`] if the bytes aren't a valid binary-AST config record — not binary-AST, the
    /// root isn't a record, or a present nested record is missing a required field (the serde reason is
    /// carried in the error).
    pub fn decode(bytes: &[u8]) -> Result<Self, ConfigError> {
        // Decode the binary-AST config record straight into the struct via serde. The Deserializer is
        // canonical-value-form-tolerant (peels a root `(: … Type)` ascription, accepts the shadowable
        // `("record" …)` head and NAME-keyed field pairs), so it reads exactly the bytes the caller's
        // `cdz convert --to bin` produces — no change to the config wire, just the reader.
        cadenza_ast_serde::from_bytes(bytes).map_err(|e| ConfigError::Decode(e.to_string()))
    }

    /// The listen address (`host:port`), defaulting to `127.0.0.1:8080`.
    #[must_use]
    pub fn listen_addr(&self) -> &str {
        self.listen.as_deref().unwrap_or("127.0.0.1:8080")
    }

    /// Assemble the configured tier stack into a single [`BlobStore`] — `[mem?, disk?, s3?]`
    /// shallowest→deepest, wrapped in a [`TieredBlobStore`]; a bare [`InMemoryBlobStore`] if nothing is
    /// configured.
    ///
    /// # Errors
    /// [`ConfigError::Tier`] if a tier fails to initialize, or [`ConfigError::S3Unsupported`] if an `s3`
    /// tier is configured but the binary lacks `--features s3`.
    pub async fn build_store(&self) -> Result<Box<dyn BlobStore>, ConfigError> {
        let mut layers: Vec<Arc<dyn BlobStore>> = Vec::new();
        if let Some(bytes) = self.mem_cache_bytes {
            layers.push(Arc::new(MemoryCache::with_capacity(bytes)));
        }
        if let Some(dc) = &self.disk_cache {
            let cache = BoundedDiskCache::open(&dc.dir, dc.bytes)
                .map_err(|e| ConfigError::Tier(format!("disk cache {}: {e}", dc.dir)))?;
            layers.push(Arc::new(cache));
        }
        self.push_s3_layer(&mut layers).await?;
        if layers.is_empty() {
            layers.push(Arc::new(InMemoryBlobStore::new()));
        }
        Ok(Box::new(TieredBlobStore::new(layers)))
    }

    #[cfg(feature = "s3")]
    async fn push_s3_layer(&self, layers: &mut Vec<Arc<dyn BlobStore>>) -> Result<(), ConfigError> {
        if let Some(s3) = &self.s3 {
            let store = crate::s3::S3BlobStore::from_config(
                &s3.bucket,
                &s3.prefix,
                s3.region.as_deref(),
                s3.endpoint.as_deref(),
            )
            .await;
            layers.push(Arc::new(store));
        }
        Ok(())
    }

    #[cfg(not(feature = "s3"))]
    async fn push_s3_layer(
        &self,
        _layers: &mut Vec<Arc<dyn BlobStore>>,
    ) -> Result<(), ConfigError> {
        if self.s3.is_some() {
            return Err(ConfigError::S3Unsupported);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cadenza_ast::ast::{Builder, CompoundCtor, IntValue, Leaf, Radix, StructId};
    use cadenza_ast::codec;
    use std::sync::Arc as StdArc;

    // ---- fixture builders ---------------------------------------------------------------------------
    fn str_field(b: &mut Builder, key: &str, value: &str) -> StructId {
        let k = b.name(key);
        let v = b.atom_leaf(Leaf::Str(StdArc::from(value)));
        b.field_pair(k, v)
    }
    fn int_field(b: &mut Builder, key: &str, value: u128) -> StructId {
        let k = b.name(key);
        let v = b.atom_leaf(Leaf::Int {
            value: IntValue::from_u128(value),
            radix: Radix::Dec,
        });
        b.field_pair(k, v)
    }
    fn record_field_pair(b: &mut Builder, key: &str, fields: &[StructId]) -> StructId {
        let k = b.name(key);
        let rec = b.compound(CompoundCtor::Record, fields);
        b.field_pair(k, rec)
    }
    fn encode_record(mut b: Builder, fields: &[StructId]) -> Vec<u8> {
        let root = b.compound(CompoundCtor::Record, fields);
        codec::encode(&b.finish(root))
    }

    #[test]
    fn decodes_a_full_config() {
        let mut b = Builder::new();
        let listen = str_field(&mut b, "listen", "0.0.0.0:9000");
        let read = str_field(&mut b, "read-credential", "r-secret");
        let write = str_field(&mut b, "write-credential", "w-secret");
        let maxb = int_field(&mut b, "max-body-bytes", 1024);
        let mem = int_field(&mut b, "mem-cache-bytes", 65536);
        let disk_dir = str_field(&mut b, "dir", "/var/cache/cas");
        let disk_bytes = int_field(&mut b, "bytes", 1_000_000);
        let disk = record_field_pair(&mut b, "disk-cache", &[disk_dir, disk_bytes]);
        let s3_bucket = str_field(&mut b, "bucket", "my-bucket");
        let s3_prefix = str_field(&mut b, "prefix", "blobs");
        let s3 = record_field_pair(&mut b, "s3", &[s3_bucket, s3_prefix]);
        let bytes = encode_record(b, &[listen, read, write, maxb, mem, disk, s3]);

        let cfg = ServerConfig::decode(&bytes).expect("decode");
        assert_eq!(cfg.listen.as_deref(), Some("0.0.0.0:9000"));
        assert_eq!(cfg.read_credential.as_deref(), Some("r-secret"));
        assert_eq!(cfg.write_credential.as_deref(), Some("w-secret"));
        assert_eq!(cfg.max_body_bytes, Some(1024));
        assert_eq!(cfg.mem_cache_bytes, Some(65536));
        let dc = cfg.disk_cache.expect("disk-cache");
        assert_eq!(dc.dir, "/var/cache/cas");
        assert_eq!(dc.bytes, 1_000_000);
        let s3 = cfg.s3.expect("s3");
        assert_eq!(s3.bucket, "my-bucket");
        assert_eq!(s3.prefix, "blobs");
        assert_eq!(s3.region, None);
    }

    #[test]
    fn decodes_a_minimal_config_and_absent_fields_are_none() {
        let mut b = Builder::new();
        let listen = str_field(&mut b, "listen", "127.0.0.1:7000");
        let bytes = encode_record(b, &[listen]);
        let cfg = ServerConfig::decode(&bytes).expect("decode");
        assert_eq!(cfg.listen_addr(), "127.0.0.1:7000");
        assert!(cfg.read_credential.is_none());
        assert!(cfg.disk_cache.is_none());
        assert!(cfg.s3.is_none());
        assert!(cfg.mem_cache_bytes.is_none());
    }

    #[test]
    fn a_default_config_listens_on_localhost() {
        let cfg = ServerConfig::default();
        assert_eq!(cfg.listen_addr(), "127.0.0.1:8080");
    }

    #[test]
    fn non_binary_ast_bytes_are_a_decode_error() {
        assert!(matches!(
            ServerConfig::decode(b"not a binary-ast document"),
            Err(ConfigError::Decode(_))
        ));
    }

    #[tokio::test]
    async fn an_empty_config_builds_an_in_memory_store() {
        let cfg = ServerConfig::default();
        // No tiers configured → a single in-memory store; a round-trip proves it's a working BlobStore.
        let store = cfg.build_store().await.expect("build");
        let bytes = bytes::Bytes::from_static(b"hi");
        let hash = store.put(bytes.clone()).await.unwrap();
        assert_eq!(store.get(hash).await.unwrap(), Some(bytes));
    }

    #[tokio::test]
    async fn a_mem_plus_disk_config_builds_a_tiered_store() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("cdz-cas-cfg-{}-{nanos}", std::process::id()));
        let cfg = ServerConfig {
            mem_cache_bytes: Some(4096),
            disk_cache: Some(DiskCacheConfig {
                dir: dir.to_string_lossy().into_owned(),
                bytes: 1_000_000,
            }),
            ..Default::default()
        };
        let store = cfg.build_store().await.expect("build");
        let payload = bytes::Bytes::from_static(b"tiered via config");
        let hash = store.put(payload.clone()).await.unwrap();
        assert_eq!(store.get(hash).await.unwrap(), Some(payload));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(not(feature = "s3"))]
    #[tokio::test]
    async fn an_s3_config_without_the_feature_is_an_error() {
        let cfg = ServerConfig {
            s3: Some(S3Config {
                bucket: "b".to_string(),
                prefix: String::new(),
                region: None,
                endpoint: None,
            }),
            ..Default::default()
        };
        assert!(matches!(
            cfg.build_store().await,
            Err(ConfigError::S3Unsupported)
        ));
    }
}
