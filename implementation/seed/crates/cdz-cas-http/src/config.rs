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
use cadenza_ast::ast::{Arenas, CompoundCtor, StructId};
use cadenza_ast::codec;
use cdz_platform::{BlobStore, InMemoryBlobStore};
use std::sync::Arc;

/// A decoded server configuration.
#[derive(Debug, Clone, Default)]
pub struct ServerConfig {
    pub listen: Option<String>,
    pub read_credential: Option<String>,
    pub write_credential: Option<String>,
    pub max_body_bytes: Option<usize>,
    pub mem_cache_bytes: Option<usize>,
    pub disk_cache: Option<DiskCacheConfig>,
    pub s3: Option<S3Config>,
}

/// The on-disk cache tier config.
#[derive(Debug, Clone)]
pub struct DiskCacheConfig {
    pub dir: String,
    pub bytes: usize,
}

/// The S3 durable-floor config.
#[derive(Debug, Clone)]
pub struct S3Config {
    pub bucket: String,
    pub prefix: String,
    pub region: Option<String>,
    pub endpoint: Option<String>,
}

/// Why a config could not be decoded or its store assembled.
#[derive(Debug)]
pub enum ConfigError {
    /// The bytes were not a valid binary-AST document.
    Decode,
    /// The document's root value is not a record.
    NotARecord,
    /// A required nested field was missing or the wrong type.
    MissingField(&'static str),
    /// A configured tier failed to initialize (e.g. the disk cache directory).
    Tier(String),
    /// The config asked for an S3 tier but the binary was built without `--features s3`.
    S3Unsupported,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode => write!(f, "config is not a valid binary-AST document"),
            Self::NotARecord => write!(f, "config root value is not a record"),
            Self::MissingField(field) => write!(f, "config missing/invalid field: {field}"),
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
    /// [`ConfigError::Decode`] if the bytes aren't binary-AST, [`ConfigError::NotARecord`] if the root isn't
    /// a record, or [`ConfigError::MissingField`] if a present nested record is missing a required field.
    pub fn decode(bytes: &[u8]) -> Result<Self, ConfigError> {
        let arenas = codec::decode(bytes).ok_or(ConfigError::Decode)?;
        let rec = unascribe(&arenas, arenas.root);
        if arenas.compound_form_of(rec, CompoundCtor::Record).is_none() {
            return Err(ConfigError::NotARecord);
        }

        let disk_cache = match record_field(&arenas, rec, "disk-cache") {
            Some(dc) => Some(DiskCacheConfig {
                dir: read_str(&arenas, dc, "dir")
                    .ok_or(ConfigError::MissingField("disk-cache.dir"))?,
                bytes: read_usize(&arenas, dc, "bytes")
                    .ok_or(ConfigError::MissingField("disk-cache.bytes"))?,
            }),
            None => None,
        };
        let s3 = match record_field(&arenas, rec, "s3") {
            Some(s) => Some(S3Config {
                bucket: read_str(&arenas, s, "bucket")
                    .ok_or(ConfigError::MissingField("s3.bucket"))?,
                prefix: read_str(&arenas, s, "prefix").unwrap_or_default(),
                region: read_str(&arenas, s, "region"),
                endpoint: read_str(&arenas, s, "endpoint"),
            }),
            None => None,
        };

        Ok(Self {
            listen: read_str(&arenas, rec, "listen"),
            read_credential: read_str(&arenas, rec, "read-credential"),
            write_credential: read_str(&arenas, rec, "write-credential"),
            max_body_bytes: read_usize(&arenas, rec, "max-body-bytes"),
            mem_cache_bytes: read_usize(&arenas, rec, "mem-cache-bytes"),
            disk_cache,
            s3,
        })
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

/// Strip a root ascription `(: value Type)` → `value` (or return `id` unchanged if it isn't one).
fn unascribe(arenas: &Arenas, id: StructId) -> StructId {
    arenas
        .as_form(id, ":")
        .and_then(|tail| tail.first().copied())
        .unwrap_or(id)
}

/// The value node of record `rec`'s field named `name`, or `None` if absent. A record is a `Record`-ctor
/// compound of `(= key value)` field pairs (ascription-tolerant, per the platform value convention).
fn record_field(arenas: &Arenas, rec: StructId, name: &str) -> Option<StructId> {
    let fields = arenas.compound_form_of(unascribe(arenas, rec), CompoundCtor::Record)?;
    fields.iter().find_map(|&f| {
        let kv = arenas.as_form(f, "=")?;
        (kv.len() == 2 && arenas.as_name(kv[0]) == Some(name)).then_some(kv[1])
    })
}

/// Read a string-valued field of `rec`.
fn read_str(arenas: &Arenas, rec: StructId, name: &str) -> Option<String> {
    arenas
        .as_str(record_field(arenas, rec, name)?)
        .map(str::to_string)
}

/// Read a non-negative-integer field of `rec` as a `usize`.
fn read_usize(arenas: &Arenas, rec: StructId, name: &str) -> Option<usize> {
    arenas
        .as_int(record_field(arenas, rec, name)?)?
        .to_u128()
        .and_then(|u| usize::try_from(u).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cadenza_ast::ast::{Builder, IntValue, Leaf, Radix, StructId};
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
            Err(ConfigError::Decode)
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
