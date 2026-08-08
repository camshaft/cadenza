//! The agent-daemon CONFIG — one TOML file that configures the whole daemon (operator directive: "set the
//! entire thing up via config and not from cli args; the only arg I want is the path to the config").
//!
//! [`DaemonConfig`] is the root: parse it from a TOML path with [`DaemonConfig::from_toml_path`], and every
//! later daemon concern reads its section — the durable-log backend (`[log]`, file dev / Dynamo prod,
//! config-SELECTABLE per the generic-async-log directive), observability (`[observability]`, the operator's
//! s2n-quic-dc-metrics), and effect-retry policy (`[retries]`). This module is the config SPINE — schema +
//! parse + validation; the wiring of each backend to real code lands as its own daemon slice. Unknown
//! fields are REJECTED (`deny_unknown_fields`) so a typo'd key is a loud config error, not silently ignored.
//!
//! **Config = INFRASTRUCTURE only, NOT a session roster** (operator refinement): the TOML configures the
//! daemon's backends/policy, but it does NOT enumerate the sessions to run. Sessions are DYNAMIC — the
//! daemon boots on its config, then sessions are INSTALLED into it at runtime (via the spawn/install path,
//! dovetailing the supervision-tree work), not read from a fixed list here.

use serde::Deserialize;
use std::path::Path;

/// The whole daemon INFRASTRUCTURE configuration, deserialized from one TOML file. Every section defaults
/// when omitted, so an empty config boots a valid daemon with dev defaults (in-memory log, metrics off,
/// default retry policy) — sessions are installed into it at runtime, not listed here.
#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct DaemonConfig {
    /// The durable event-log backend (where each session's log persists). Defaults to in-memory (dev).
    #[serde(default)]
    pub log: LogConfig,
    /// Observability / metrics. Defaults to disabled.
    #[serde(default)]
    pub observability: ObservabilityConfig,
    /// Effect-retry policy the daemon's supervisor applies to a RETRYABLE effect outcome. Defaults to a
    /// bounded exponential backoff.
    #[serde(default)]
    pub retries: RetryConfig,
    /// The admin control interface (the Unix-domain socket an admin sends commands to). Defaults to
    /// disabled — a daemon with no admin section boots without a control socket.
    #[serde(default)]
    pub admin: AdminConfig,
    /// The reducer BLOB store — where the daemon fetches a session's reducer component by content hash when
    /// an admin `install-session` names one. Defaults to in-memory (dev/test: nothing persisted, an install
    /// only works for a reducer already `put` into the store this run). A real deployment sets a `dir`.
    #[serde(default)]
    pub blob: BlobConfig,
    /// The reducer-blob CACHE (S3-FIFO, in front of `blob`). Defaults to ON (128 MiB in-memory, 2 GiB disk);
    /// a `0` per-tier budget disables that tier. Content-addressed blobs are immutable → cache hits are always
    /// correct with no invalidation.
    #[serde(default)]
    pub blob_cache: BlobCacheConfig,
    /// The canonical §4c NAME STORE durability backend — where the daemon snapshots its shared name directory
    /// on mutation + restores it on boot (AWS-backends arc I4a). Defaults to `memory` (no durability — the
    /// daemon boots share-less, today's behavior). `s3` makes the canonical store DURABLE across a restart.
    #[serde(default)]
    pub name_store: NameStoreConfig,
    /// The SESSION REGISTRY backend — the index of which sessions exist + which are ACTIVE, so daemon boot-
    /// recovery enumerates recoverable sessions (I4b) + operators query live sessions WITHOUT scanning the
    /// event log. Defaults to `memory` (no durable registry — the daemon boots with an empty registry,
    /// today's lossy-on-restart behavior). `dynamo` makes the registry DURABLE (a session-per-item table +
    /// status GSI). Boot-recovery only runs when BOTH this AND a durable `[log]` backend are configured.
    #[serde(default)]
    pub session_registry: SessionRegistryConfig,
    /// Structured TRACING — spans/events on the host loop + session/effect surface (operator directive:
    /// "add tracing to the harness, configured via the toml"). Defaults to disabled (no subscriber
    /// installed). The subscriber the daemon initializes from this section is wired in a following slice;
    /// this section always parses ahead of that (config first, subscriber follows — like `[observability]`).
    #[serde(default)]
    pub tracing: TracingConfig,
}

/// The durable-log backend — config-SELECTABLE (the operator's "selectable backends"): `memory` for dev,
/// `file` for a single-host durable log, `dynamo` for a distributed prod log. The variant's own fields
/// carry its parameters (a path for file, a table for dynamo).
#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
#[serde(tag = "backend", rename_all = "kebab-case", deny_unknown_fields)]
pub enum LogConfig {
    /// In-memory log — non-durable, dev/test only (lost on restart). The default.
    #[default]
    Memory,
    /// File-backed durable log under `dir` — the single-host durable backend.
    File { dir: String },
    /// DynamoDB-backed durable log in `table` (region from the ambient AWS env, like the Bedrock creds).
    /// The distributed prod target the operator flagged.
    Dynamo { table: String },
}

/// Observability config — the daemon's metrics integration (the operator's `s2n-quic-dc-metrics` crate,
/// `camshaft/s2n-quic` branch `main`). Disabled by default. This section describes what the emitter does
/// WHEN WIRED: when `enabled`, metrics fan out to the configured `targets` — a LIST so MULTIPLE backends are
/// supported (the operator's config-multiple requirement), with at least one required when enabled
/// (validated below). Each `[[observability.target]]` is a [`MetricsTarget`] whose `kind` selects a backend
/// and whose OTHER fields are that backend's OWN typed config, so the wired emitter can send to several
/// sinks (multiple statsd collectors, or different backend types).
///
/// **Current daemon behavior:** the daemon reads `observability.enabled` (it logs it at boot). The `targets`
/// are always PARSED (serde), but only VALIDATED (non-empty list, non-blank endpoints) WHEN `enabled` — a
/// disabled section with a blank endpoint parses fine (nothing consumes it). The daemon does NOT emit to the
/// targets at all: it never reads them beyond that gated validation, so no metrics are sent anywhere
/// regardless of config. The emitter that reads `targets` + performs the fan-out is a separate, feature-gated
/// component (kept out of the default build to exclude the QUIC/metrics dependency tree); the fan-out
/// described above is that emitter's intended behavior, which the config is defined ahead of.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ObservabilityConfig {
    /// Emit metrics. Default off. When true, `targets` must be non-empty (validated below).
    #[serde(default)]
    pub enabled: bool,
    /// The metrics target backends to FAN OUT to — a LIST (not a single endpoint) so metrics can go to
    /// several sinks (the operator's config-MULTIPLE requirement). TOML: `[[observability.target]]` tables.
    #[serde(default, rename = "target")]
    pub targets: Vec<MetricsTarget>,
    /// How often (seconds) the daemon reports/flushes metrics to the backends — the exporter's periodic
    /// `Registry::report` cadence. Default 60 (the conventional statsd/otlp push interval). Since the
    /// registry Counters are DRAIN-ON-REPORT, this interval IS the aggregation window (each report emits the
    /// per-interval delta — standard push semantics). Validated ONLY when `enabled`: 0 is REJECTED (a config
    /// error — a 0 interval would spin a tight flush loop); when disabled it isn't checked (nothing reports).
    /// Applies to all targets (a per-target override could be added later).
    #[serde(default = "ObservabilityConfig::default_report_interval_secs")]
    pub report_interval_secs: u64,
}

impl ObservabilityConfig {
    fn default_report_interval_secs() -> u64 {
        60
    }
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        ObservabilityConfig {
            enabled: false,
            targets: Vec::new(),
            report_interval_secs: Self::default_report_interval_secs(),
        }
    }
}

/// One metrics target backend — a per-kind TYPED config, NOT a generic `{kind, endpoint}` pair (operator
/// directive: "each backend needs its own config fields - not just some generic target string"). `kind`
/// discriminates the variant (a tagged union, like [`LogConfig`]/[`BlobConfig`]); each variant carries the
/// fields THAT backend needs, validated per-kind. Adding a backend adds a typed variant with its own fields,
/// so a mis-shaped target (a field that belongs to another backend, a missing required field) is a loud
/// deser error rather than an ignored generic string.
///
/// The variants model REAL `s2n-quic-dc-metrics` backends. Modeled today: the two PUSH backends — `statsd`
/// (a UDP sink) and `otlp` (an OTLP-payload sink) — both a caller-supplied sink the daemon flushes to, so
/// their config is an `endpoint` the daemon sends to.
///
/// The crate's other backends are NOT modeled yet because their config SHAPE differs from the push kinds:
/// `prometheus` is PULL (the backend exposes state an external scraper reads — its config would be a scrape
/// bind address, and the daemon would run an inbound listener), and `querylog` / `arrow` are FILE (local
/// output the daemon writes — config would be a path/rotation). Each is a cheap additive variant once its
/// shape is settled; they're absent here rather than modeled with a guessed shape.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum MetricsTarget {
    /// The `statsd` backend — `s2n-quic-dc-metrics`' `StatsdBackend`. The daemon's statsd sink sends UDP
    /// payloads to the collector at `endpoint`; `prefix` optionally namespaces every metric name (the
    /// backend's `prefix` argument), e.g. `"cdz.agent"`.
    Statsd {
        /// The statsd collector address the sink sends UDP payloads to (`host:port`).
        endpoint: String,
        /// Optional metric-name prefix applied to every emitted metric (maps to `StatsdBackend`'s prefix).
        #[serde(default)]
        prefix: Option<String>,
    },
    /// The `otlp` backend — `s2n-quic-dc-metrics`' `OtlpBackend`. Same PUSH shape as statsd: the daemon's
    /// OTLP sink sends `ExportMetricsServiceRequest` payloads to the collector at `endpoint`. `scope_name` /
    /// `scope_version` populate the OTLP `InstrumentationScope` on every metric (the backend's `new`
    /// arguments). Both are optional; a minimal config is just an endpoint.
    Otlp {
        /// The OTLP collector endpoint the sink sends to (`host:port` or a URL, per the sink transport).
        endpoint: String,
        /// The OTLP instrumentation-scope name embedded in every metric. Omitting it leaves it `None` here;
        /// the emitter/backend wiring supplies the default (e.g. the crate name) — config parsing does not.
        #[serde(default)]
        scope_name: Option<String>,
        /// The OTLP instrumentation-scope version. Omitting it leaves it `None` here; the emitter/backend
        /// wiring decides any default — config parsing does not.
        #[serde(default)]
        scope_version: Option<String>,
    },
    /// The `cloudwatch` backend — the AWS-native metrics export (I3). Like statsd/otlp it's a PUSH backend,
    /// but the "endpoint" is the AWS CloudWatch `PutMetricData` API (region + credentials from the SDK default
    /// provider chain — env/profile/IMDS, no endpoint field needed), so its config is a metric `namespace` (the
    /// CloudWatch scope every published datum lands under), NOT a push endpoint. `namespace` is REQUIRED and
    /// validated non-empty. Exportable only when the `metrics-export-cloudwatch` feature is compiled; the
    /// variant is always PARSED (feature-independent, like the others), and a build without the feature reports
    /// it as unexportable at boot rather than silently no-op'ing.
    #[serde(rename = "cloudwatch")]
    CloudWatch {
        /// The CloudWatch metric namespace every datum is published under (e.g. `"CDZ/AgentHost"`).
        namespace: String,
    },
    /// The `prometheus` backend — `s2n-quic-dc-metrics`' `PrometheusBackend`. UNLIKE statsd/otlp (outbound
    /// PUSH), prometheus is PULL: the daemon SERVES an HTTP scrape endpoint (`GET /metrics`) that a scraper
    /// reads, so its config is a `bind` LISTEN address (not a push endpoint). `bind` is REQUIRED (no default),
    /// forcing a conscious address choice (no silent exposure). This is a NO-AUTH endpoint, so the posture
    /// (concierge ruling) is: a LOOPBACK bind just works, but a NON-loopback bind is REJECTED at validation
    /// unless `allow_non_loopback = true` — a deliberate second acknowledgment that you're exposing an
    /// unauthenticated inbound metrics port. `prefix` optionally namespaces every metric.
    Prometheus {
        /// The address the scrape listener binds (`host:port`), e.g. `"127.0.0.1:9090"`. A loopback host keeps
        /// the no-auth endpoint on the local trust boundary; a non-loopback host requires `allow_non_loopback`.
        bind: String,
        /// Opt-in to a NON-loopback `bind` for this unauthenticated scrape endpoint. Default `false`: a
        /// non-loopback bind without this set is a validation error (an unauthenticated public metrics port
        /// must be chosen by INTENT, not reachable by omission). A non-loopback bind WITH it set is still
        /// warned at boot. Loopback binds ignore this flag.
        #[serde(default)]
        allow_non_loopback: bool,
        /// Optional metric-name prefix applied to every emitted metric (maps to `PrometheusBackend`'s prefix).
        #[serde(default)]
        prefix: Option<String>,
    },
}

impl MetricsTarget {
    /// The backend kind token (matches the TOML `kind = "…"` tag) — for diagnostics + fan-out logging.
    pub fn kind(&self) -> &'static str {
        match self {
            MetricsTarget::Statsd { .. } => "statsd",
            MetricsTarget::Otlp { .. } => "otlp",
            MetricsTarget::CloudWatch { .. } => "cloudwatch",
            MetricsTarget::Prometheus { .. } => "prometheus",
        }
    }
}

/// The admin control interface config — the local Unix-domain socket an admin communicates with the
/// running daemon over (install/list/status/stop sessions). Disabled by default; when enabled, `socket` is
/// the filesystem path to bind (validated present below). LOCAL only — a Unix socket, not a network
/// listener, matching the hermetic/no-egress posture (a remote admin transport would be a later opt-in with
/// its own auth). The socket LISTENER itself is behind the `admin` cargo feature; this config is always
/// parseable so a config-path smoke test needs no feature.
#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct AdminConfig {
    /// Serve the admin control interface. Default off (no control socket).
    #[serde(default)]
    pub enabled: bool,
    /// The Unix-domain socket path to bind (e.g. `/run/cdz/admin.sock`); required when `enabled` —
    /// validated below.
    #[serde(default)]
    pub socket: Option<String>,
}

/// The reducer blob-store backend — where the daemon resolves a `reducer_hash` to component bytes on an
/// admin `install-session`. `memory` (default) is a non-durable in-process store (dev/test); `dir` is a
/// content-addressed disk store rooted at a path (the single-host durable backend). The
/// [`BlobStore`](cdz_kernel::blob::BlobStore) trait is the extension seam — further backends drop in behind
/// it without a config reshape.
///
/// **Wired.** The daemon bin reads `config.blob` at boot and builds the selected store — `memory` →
/// [`MemBlobStore`](cdz_kernel::blob::MemBlobStore), `dir` → [`DiskBlobStore`](cdz_kernel::blob::DiskBlobStore)
/// rooted at the path — handing it to the [`ComponentSessionFactory`](crate::ComponentSessionFactory) that
/// resolves each `install-session`'s `reducer_hash`. (This closed the earlier staging gap from #1981 where the
/// section parsed ahead of the factory-wiring slice; `backend = "dir"` now takes effect.)
#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
#[serde(tag = "backend", rename_all = "kebab-case", deny_unknown_fields)]
pub enum BlobConfig {
    /// In-memory blob store — non-durable, dev/test (an install only finds a reducer `put` this run). Default.
    #[default]
    Memory,
    /// Disk-backed content-addressed store rooted at `dir` — the single-host durable reducer store.
    Dir { dir: String },
    /// S3-backed content-addressed store — the AWS-native production reducer store (blobs live in `bucket`
    /// under an optional key `prefix`). Wired only when the `live-aws-storage` feature is compiled (the daemon
    /// selects [`S3BlobStore`](crate::S3BlobStore)); parsing/validation is always-on (infra-core), so a config
    /// naming `backend = "s3"` validates regardless of feature, and a build WITHOUT `live-aws-storage` reports a
    /// clean "not compiled in" error at boot rather than silently falling back.
    S3 {
        bucket: String,
        #[serde(default)]
        prefix: String,
    },
}

/// The canonical §4c NAME STORE durability backend — how the daemon persists its shared name directory so
/// it survives a restart (AWS-backends arc I4a). UNLIKE [`BlobConfig`] (a content-addressed store), this is
/// a FIXED-location snapshot: the daemon snapshots the canonical store on mutation + restores it on boot from
/// a known location. `memory` (default) = NO durability (the daemon boots share-less, today's behavior); `s3`
/// = a durable canonical store snapshotted to a fixed S3 key under `bucket`/`prefix`. Selected backend wired
/// only when its feature is compiled: the `s3` RUNTIME backend
/// ([`S3NameStoreSnapshot`](crate::S3NameStoreSnapshot)) needs `live-aws-storage`; parsing/validation is
/// always-on (infra-core), so a config naming `backend = "s3"` validates regardless of feature, and a build
/// WITHOUT `live-aws-storage` reports a clean "not compiled in" error at boot rather than silently degrading.
#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
#[serde(tag = "backend", rename_all = "kebab-case", deny_unknown_fields)]
pub enum NameStoreConfig {
    /// No durability — the daemon boots WITHOUT a canonical name store (share-less, today's behavior). The
    /// default: a daemon with no `[name_store]` section keeps its existing share-less shape. (`memory` names
    /// the "in-memory / non-durable" intent uniformly with [`BlobConfig::Memory`] / [`LogConfig::Memory`].)
    #[default]
    Memory,
    /// S3-backed durable canonical store — snapshotted to a FIXED key under `bucket` (+ an optional key
    /// `prefix`) on mutation, restored from it on boot. The AWS-native production name directory. Wired only
    /// when `live-aws-storage` is compiled; validation (non-empty bucket) is always-on.
    S3 {
        bucket: String,
        #[serde(default)]
        prefix: String,
    },
}

/// The SESSION REGISTRY durability backend (I4b) — the index of which sessions exist + their lifecycle status
/// (active / terminated), so the daemon can (a) BOOT-RECOVER durably-logged sessions after a restart
/// (enumerate → recover → re-register) and (b) answer "current active sessions" — both WITHOUT an O(all-events)
/// scan of the event log (operator: "build an index, do it right"). `memory` (default) = NO durable registry
/// (the daemon boots with an empty in-memory registry, today's lossy-on-restart behavior — nothing to
/// recover); `dynamo` = a durable DynamoDB registry ([`DynamoSessionRegistry`](crate::DynamoSessionRegistry) —
/// a session-per-item `table` + a status GSI). Selected backend wired only when its feature is compiled: the
/// `dynamo` RUNTIME backend needs `live-aws-storage`; parsing/validation is always-on (infra-core), so a config
/// naming `backend = "dynamo"` validates regardless of feature, and a build WITHOUT `live-aws-storage` reports
/// a clean "not compiled in" error at boot rather than silently degrading. Boot-recovery runs only when this is
/// `dynamo` AND `[log]` is durable (a session's log is what recovery replays).
#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
#[serde(tag = "backend", rename_all = "kebab-case", deny_unknown_fields)]
pub enum SessionRegistryConfig {
    /// No durable registry — the daemon boots with an empty in-memory session registry (today's behavior:
    /// nothing to boot-recover, sessions are lost on restart). The default (uniform `memory` naming with the
    /// other backends' non-durable variant).
    #[default]
    Memory,
    /// DynamoDB-backed durable session registry in `table` (region from the ambient AWS env, like the log /
    /// Bedrock backends). One item per session (pk = session_id) + a `status`-index GSI for the active-sessions
    /// query. Wired only when `live-aws-storage` is compiled; validation (non-empty table) is always-on.
    Dynamo { table: String },
}

/// Effect-retry policy: a bounded exponential backoff the daemon's supervisor applies to a RETRYABLE
/// effect outcome (a transient Bedrock/HTTP failure the host classified `Retryability::Retryable`). A
/// PERMANENT failure is never retried. This is the POLICY; the retry MECHANISM (re-emit via Timer on a
/// RETRYABLE `EffectResult`) is a userspace/kernel-supervisor concern that reads it. The policy math —
/// [`should_retry`](Self::should_retry) (attempt within `max_attempts`?) + [`backoff_ms`](Self::backoff_ms)
/// (the bounded-exponential delay before an attempt) — lives here so every retry driver computes it
/// identically (and it's unit-tested without needing the supervision mechanism).
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RetryConfig {
    /// Max retry attempts for one effect before giving up (0 = never retry). Default 3.
    #[serde(default = "RetryConfig::default_max_attempts")]
    pub max_attempts: u32,
    /// Base backoff in milliseconds (the first retry waits this long; each subsequent doubles). Default 100.
    #[serde(default = "RetryConfig::default_base_ms")]
    pub base_ms: u64,
    /// Cap on any single backoff delay in milliseconds (exponential growth saturates here). Default 30000.
    #[serde(default = "RetryConfig::default_max_ms")]
    pub max_ms: u64,
}

impl RetryConfig {
    fn default_max_attempts() -> u32 {
        3
    }
    fn default_base_ms() -> u64 {
        100
    }
    fn default_max_ms() -> u64 {
        30_000
    }

    /// Should a RETRYABLE failure be retried on its `attempt`-th retry? `attempt` is 1-based (the FIRST
    /// retry is `attempt = 1`). True while `attempt <= max_attempts`; `max_attempts = 0` never retries.
    /// The retry driver (a userspace/kernel supervisor) calls this to decide whether to schedule another
    /// re-emit vs give up + escalate. (A PERMANENT failure is never passed here — it's not retried at all.)
    pub fn should_retry(&self, attempt: u32) -> bool {
        attempt >= 1 && attempt <= self.max_attempts
    }

    /// The backoff delay (ms) before the `attempt`-th retry — bounded exponential: `base_ms * 2^(attempt-1)`,
    /// saturating at `max_ms`. `attempt` is 1-based (the first retry waits `base_ms`, the second `2*base_ms`,
    /// …). The retry driver waits this long (a Timer) before re-emitting the effect with the same
    /// idempotency key (§16c — a paid Bedrock invoke dedups on re-issue). Saturating throughout, so a large
    /// attempt can't overflow — it just pins at `max_ms`.
    ///
    /// `attempt = 0` (or below 1) returns 0 (no wait) — a caller shouldn't ask for a pre-first-retry delay,
    /// but returning 0 is the harmless total answer rather than a panic (§17 totality).
    pub fn backoff_ms(&self, attempt: u32) -> u64 {
        if attempt < 1 {
            return 0;
        }
        // 2^(attempt-1) via saturating shift: a shift >= 64 (attempt-1 >= 64) is out of range for a u64, so
        // clamp the exponent and let the multiply saturate to max_ms anyway.
        let exp = attempt - 1;
        // A `1u64 << exp` is out of range only at exp >= 64 (in safe Rust that PANICS in debug / masks in
        // release — not UB, but still wrong); exp == 63 (1<<63 = 2^63) is valid, so guard at the exact
        // boundary. (Reachable behavior is unchanged either way — at exp 63 the saturating_mul+min already
        // pin at max_ms for any real config — but this makes the boundary precisely correct; #1998 review.)
        let factor: u64 = if exp >= 64 { u64::MAX } else { 1u64 << exp };
        let delay = self.base_ms.saturating_mul(factor);
        delay.min(self.max_ms)
    }
}

impl Default for RetryConfig {
    fn default() -> Self {
        RetryConfig {
            max_attempts: Self::default_max_attempts(),
            base_ms: Self::default_base_ms(),
            max_ms: Self::default_max_ms(),
        }
    }
}

/// The reducer-blob CACHE in front of the [`BlobConfig`] backing store (operator directive: "for the blob
/// store we should have a bounded cache … multi-tier to disk … S3-FIFO"). Content-addressed blobs are
/// immutable, so a cache hit is always correct with no invalidation; the S3-FIFO eviction resists the
/// one-hit-wonder pollution a plain LRU suffers. ON by default (a bounded cache is strictly beneficial); a
/// `0` byte budget disables that tier.
///
/// **Daemon behavior:** the in-MEMORY tier ([`mem_bytes`](Self::mem_bytes)) wraps the configured backing store
/// in a [`CachingBlobStore`](crate::CachingBlobStore). The on-DISK tier ([`disk_bytes`](Self::disk_bytes) +
/// [`disk_dir`](Self::disk_dir)) sits BETWEEN memory and the backing store — mem over disk over S3 — via a
/// [`DiskCacheTier`](crate::DiskCacheTier), enabled only when `disk_bytes > 0` AND a `disk_dir` is set (a disk
/// cache needs an explicit directory; there's no sensible default location).
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BlobCacheConfig {
    /// In-memory cache byte budget (S3-FIFO, small+main). `0` disables the in-memory cache (the backing store
    /// is used directly). Default 128 MiB — hot in-memory, safe on a modest host.
    #[serde(default = "BlobCacheConfig::default_mem_bytes")]
    pub mem_bytes: usize,
    /// On-disk cache byte budget (the tier between memory and the backing store). `0` disables the disk tier.
    /// Default 2 GiB. The disk tier is wired only when this is `> 0` AND `disk_dir` is set.
    #[serde(default = "BlobCacheConfig::default_disk_bytes")]
    pub disk_bytes: usize,
    /// On-disk cache DIRECTORY (content-hash-named files live here). `None` (the default) disables the disk
    /// tier regardless of `disk_bytes` — a disk cache needs an explicit path, so there's no default location.
    /// A deployment that wants the disk tier sets both this and `disk_bytes`.
    #[serde(default)]
    pub disk_dir: Option<String>,
}

impl BlobCacheConfig {
    fn default_mem_bytes() -> usize {
        128 * 1024 * 1024
    }
    fn default_disk_bytes() -> usize {
        2 * 1024 * 1024 * 1024
    }
}

impl Default for BlobCacheConfig {
    fn default() -> Self {
        BlobCacheConfig {
            mem_bytes: Self::default_mem_bytes(),
            disk_bytes: Self::default_disk_bytes(),
            disk_dir: None,
        }
    }
}

/// Structured-tracing config — how the daemon initializes its `tracing` subscriber (operator: "add tracing
/// … configured via the toml"). Disabled by default (no subscriber installed → spans/events are the
/// near-zero-cost no-op the `tracing` facade compiles to when nothing subscribes). When `enabled`, the
/// daemon installs a `tracing-subscriber` at boot with this `filter`/`format`/`output`.
///
/// **Config first, subscriber follows.** This section always parses; the daemon-bin code that reads it to
/// install a subscriber is the following slice (staged like `[observability]`). `filter` uses the standard
/// `EnvFilter` directive syntax (e.g. `"info"`, `"cdz_agent_host=debug,warn"`) — the same grammar as
/// `RUST_LOG` — so an operator writes familiar per-target levels.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TracingConfig {
    /// Install a tracing subscriber. Default off — a daemon with no `[tracing]` section (or `enabled=false`)
    /// runs with no subscriber, so the facade's spans/events are inert.
    #[serde(default)]
    pub enabled: bool,
    /// The `EnvFilter` directive string (RUST_LOG grammar) selecting which spans/events are recorded. Default
    /// `"info"`. Validated non-blank when `enabled`.
    #[serde(default = "TracingConfig::default_filter")]
    pub filter: String,
    /// The event/span rendering format. Default `compact`.
    #[serde(default)]
    pub format: TracingFormat,
    /// Where formatted output goes. Default `stderr` (diagnostics off stdout, matching the daemon's boot
    /// eprintln! convention). A `file` output writes to `path`.
    #[serde(default)]
    pub output: TracingOutput,
}

impl TracingConfig {
    fn default_filter() -> String {
        "info".to_string()
    }
}

impl Default for TracingConfig {
    fn default() -> Self {
        TracingConfig {
            enabled: false,
            filter: Self::default_filter(),
            format: TracingFormat::default(),
            output: TracingOutput::default(),
        }
    }
}

/// The tracing event/span rendering format — maps to a `tracing-subscriber` fmt layer style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum TracingFormat {
    /// Terse single-line events (the fmt layer's `.compact()`). Default.
    #[default]
    Compact,
    /// Multi-line human-readable (`.pretty()`).
    Pretty,
    /// Structured JSON (`.json()`) — for machine ingestion / an OTLP-adjacent pipeline.
    Json,
}

/// Where the tracing subscriber writes — a `stderr`/`stdout` stream or a `file` at a path.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(tag = "to", rename_all = "kebab-case", deny_unknown_fields)]
pub enum TracingOutput {
    /// Write to stderr — the default (keeps trace output off stdout, matching the daemon's boot diagnostics).
    #[default]
    Stderr,
    /// Write to stdout.
    Stdout,
    /// Append to a file at `path` (created if absent). The daemon opens it at boot; an un-openable path is a
    /// fatal config error (surfaced at boot, like a bad `[log]` dir).
    File { path: String },
}

/// A config error — a bad path, unparseable TOML, or a semantic-validation failure. Stringly-typed (the
/// daemon surfaces it to stderr + exits non-zero; there's no recovery from a bad config).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "daemon config error: {}", self.0)
    }
}
impl std::error::Error for ConfigError {}

/// `true` only if `addr` (`host:port`) resolves to at least one address and EVERY resolved address is
/// loopback. FAIL-CLOSED: an unresolvable/unparseable/empty address returns `false` (treated as non-loopback),
/// and — critically — a name resolving to a MIX of loopback + non-loopback addresses is NOT loopback. Checking
/// only the FIRST resolved address would let a mixed name (loopback first) pass the gate while
/// `TcpListener::bind` could still bind the non-loopback address (a bypass of the `allow_non_loopback` opt-in;
/// same multi-address class as the #2112 IPv6-first bind — #2160 review). Kept in `config` (always-on) so
/// validation enforces the prometheus posture without the `metrics-export-prometheus` feature (which has its
/// own runtime copy for the boot warning).
fn bind_is_loopback(addr: &str) -> bool {
    use std::net::ToSocketAddrs;
    match addr.to_socket_addrs() {
        Ok(mut addrs) => {
            let mut any = false;
            for a in &mut addrs {
                any = true;
                if !a.ip().is_loopback() {
                    return false; // a mixed name with any non-loopback addr is NOT loopback
                }
            }
            any // all resolved were loopback AND at least one existed
        }
        Err(_) => false, // unresolvable → fail-closed (needs the explicit opt-in)
    }
}

impl DaemonConfig {
    /// Load + validate the daemon config from a TOML file at `path` — the daemon's single boot input.
    pub fn from_toml_path(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .map_err(|e| ConfigError(format!("cannot read config {}: {e}", path.display())))?;
        Self::from_toml_str(&text)
    }

    /// Parse + validate from a TOML string (the testable core of [`from_toml_path`](Self::from_toml_path)).
    pub fn from_toml_str(text: &str) -> Result<Self, ConfigError> {
        let cfg: DaemonConfig =
            // `toml::from_str` covers BOTH invalid TOML syntax AND a valid-TOML-but-invalid-schema failure
            // (an unknown key under deny_unknown_fields, a type mismatch); its own error text says which, so
            // the wording is "could not parse daemon config" rather than mislabeling a schema error as a
            // syntax error.
            toml::from_str(text)
                .map_err(|e| ConfigError(format!("could not parse daemon config: {e}")))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Semantic validation beyond what the type system + serde enforce: non-empty selected-backend paths
    /// and an observability endpoint present when enabled. (No session checks — sessions are installed at
    /// runtime, not configured here.)
    fn validate(&self) -> Result<(), ConfigError> {
        match &self.log {
            LogConfig::File { dir } if dir.trim().is_empty() => {
                return Err(ConfigError(
                    "[log] backend=file needs a non-empty dir".into(),
                ));
            }
            LogConfig::Dynamo { table } if table.trim().is_empty() => {
                return Err(ConfigError(
                    "[log] backend=dynamo needs a non-empty table".into(),
                ));
            }
            _ => {}
        }
        if self.observability.enabled {
            if self.observability.targets.is_empty() {
                return Err(ConfigError(
                    "[observability] enabled=true needs at least one [[observability.target]]"
                        .into(),
                ));
            }
            if self.observability.report_interval_secs == 0 {
                return Err(ConfigError(
                    "[observability] report_interval_secs must be >= 1 (0 would spin a tight flush loop)"
                        .into(),
                ));
            }
            // Per-kind semantic validation: each variant checks the fields IT needs (serde already rejected a
            // missing/unknown field or a bad `kind`; this catches present-but-blank values).
            for (i, t) in self.observability.targets.iter().enumerate() {
                match t {
                    // Both PUSH backends require a non-empty endpoint (the collector the sink sends to).
                    MetricsTarget::Statsd { endpoint, .. }
                    | MetricsTarget::Otlp { endpoint, .. }
                        if endpoint.trim().is_empty() =>
                    {
                        return Err(ConfigError(format!(
                            "[observability] target {i} ({}) needs a non-empty endpoint",
                            t.kind()
                        )));
                    }
                    // The CloudWatch PUSH backend requires a non-empty namespace (its metric scope; there's no
                    // endpoint — region/creds come from the SDK env).
                    MetricsTarget::CloudWatch { namespace } if namespace.trim().is_empty() => {
                        return Err(ConfigError(format!(
                            "[observability] target {i} (cloudwatch) needs a non-empty namespace"
                        )));
                    }
                    // The PULL backend requires a non-empty bind (LISTEN) address.
                    MetricsTarget::Prometheus { bind, .. } if bind.trim().is_empty() => {
                        return Err(ConfigError(format!(
                            "[observability] target {i} (prometheus) needs a non-empty bind address"
                        )));
                    }
                    // A non-loopback bind for this UNAUTHENTICATED scrape endpoint is rejected unless the
                    // operator explicitly opts in (concierge posture: an unauth public metrics port must be
                    // chosen by intent, not reachable by omission). A bind that can't be resolved to classify
                    // is treated as non-loopback (fail-closed) — it still needs the opt-in.
                    MetricsTarget::Prometheus {
                        bind,
                        allow_non_loopback,
                        ..
                    } if !allow_non_loopback && !bind_is_loopback(bind) => {
                        return Err(ConfigError(format!(
                            "[observability] target {i} (prometheus) binds a non-loopback address {bind:?} \
                             for an UNAUTHENTICATED scrape endpoint — set allow_non_loopback = true to opt in \
                             deliberately, or bind a loopback address (e.g. 127.0.0.1:PORT)"
                        )));
                    }
                    _ => {}
                }
            }
        }
        match &self.admin {
            AdminConfig {
                enabled: true,
                socket: None,
            } => {
                return Err(ConfigError(
                    "[admin] enabled=true needs a socket path".into(),
                ));
            }
            AdminConfig {
                enabled: true,
                socket: Some(s),
            } if s.trim().is_empty() => {
                return Err(ConfigError(
                    "[admin] enabled=true needs a non-empty socket path".into(),
                ));
            }
            _ => {}
        }
        if let BlobConfig::Dir { dir } = &self.blob {
            if dir.trim().is_empty() {
                return Err(ConfigError(
                    "[blob] backend=dir needs a non-empty dir".into(),
                ));
            }
        }
        if let BlobConfig::S3 { bucket, .. } = &self.blob {
            if bucket.trim().is_empty() {
                return Err(ConfigError(
                    "[blob] backend=s3 needs a non-empty bucket".into(),
                ));
            }
        }
        if let NameStoreConfig::S3 { bucket, .. } = &self.name_store {
            if bucket.trim().is_empty() {
                return Err(ConfigError(
                    "[name_store] backend=s3 needs a non-empty bucket".into(),
                ));
            }
        }
        if let SessionRegistryConfig::Dynamo { table } = &self.session_registry {
            if table.trim().is_empty() {
                return Err(ConfigError(
                    "[session_registry] backend=dynamo needs a non-empty table".into(),
                ));
            }
        }
        if self.tracing.enabled {
            if self.tracing.filter.trim().is_empty() {
                return Err(ConfigError(
                    "[tracing] enabled=true needs a non-empty filter (e.g. \"info\")".into(),
                ));
            }
            if let TracingOutput::File { path } = &self.tracing.output {
                if path.trim().is_empty() {
                    return Err(ConfigError(
                        "[tracing] output to=file needs a non-empty path".into(),
                    ));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_config_boots_a_valid_daemon_with_defaults() {
        // Config is INFRASTRUCTURE only — no session roster. An empty config is valid (sessions install at
        // runtime); every section takes its dev default.
        let cfg = DaemonConfig::from_toml_str("").expect("an empty config is a valid daemon");
        assert_eq!(cfg, DaemonConfig::default());
        assert_eq!(cfg.log, LogConfig::Memory);
        assert!(!cfg.observability.enabled);
        assert_eq!(
            cfg.observability.report_interval_secs, 60,
            "default report interval"
        );
        assert_eq!(cfg.retries, RetryConfig::default());
        // [session_registry] default = memory (no durable registry, lossy-on-restart).
        assert_eq!(cfg.session_registry, SessionRegistryConfig::Memory);
        // [tracing] defaults: disabled, "info" filter, compact/stderr.
        assert!(!cfg.tracing.enabled);
        assert_eq!(cfg.tracing.filter, "info");
        assert_eq!(cfg.tracing.format, TracingFormat::Compact);
        assert_eq!(cfg.tracing.output, TracingOutput::Stderr);
    }

    #[test]
    fn a_full_config_selects_each_backend() {
        let cfg = DaemonConfig::from_toml_str(
            r#"
            [log]
            backend = "dynamo"
            table = "cdz-agent-log"

            [observability]
            enabled = true
            [[observability.target]]
            kind = "statsd"
            endpoint = "127.0.0.1:9000"

            [session_registry]
            backend = "dynamo"
            table = "cdz-session-registry"

            [retries]
            max_attempts = 5
            base_ms = 200
            max_ms = 10000
            "#,
        )
        .expect("full config parses");
        assert_eq!(
            cfg.log,
            LogConfig::Dynamo {
                table: "cdz-agent-log".into()
            }
        );
        assert_eq!(
            cfg.session_registry,
            SessionRegistryConfig::Dynamo {
                table: "cdz-session-registry".into()
            }
        );
        assert!(cfg.observability.enabled);
        assert_eq!(cfg.observability.targets.len(), 1);
        assert_eq!(
            cfg.observability.targets[0],
            MetricsTarget::Statsd {
                endpoint: "127.0.0.1:9000".into(),
                prefix: None,
            }
        );
        assert_eq!(cfg.observability.targets[0].kind(), "statsd");
        assert_eq!(cfg.retries.max_attempts, 5);
    }

    #[test]
    fn session_registry_dynamo_needs_a_nonempty_table() {
        // The registry Dynamo variant parses + validates like the log dynamo backend; an empty table is a
        // clean config error (surfaced at boot), not a silent misconfig.
        // from_toml_str validates internally, so a non-empty table parses + validates clean...
        let ok = DaemonConfig::from_toml_str(
            r#"
            [session_registry]
            backend = "dynamo"
            table = "cdz-session-registry"
            "#,
        )
        .expect("a dynamo registry with a non-empty table parses + validates");
        assert!(matches!(
            ok.session_registry,
            SessionRegistryConfig::Dynamo { .. }
        ));

        // ...and an empty table is rejected by the internal validate() (surfaced from from_toml_str).
        let err = DaemonConfig::from_toml_str(
            r#"
            [session_registry]
            backend = "dynamo"
            table = ""
            "#,
        )
        .expect_err("an empty registry table is rejected at parse + validate");
        assert!(
            format!("{err}").contains("[session_registry] backend=dynamo needs a non-empty table"),
            "clear registry-table error: {err}"
        );
    }

    #[test]
    fn a_file_log_backend_carries_its_dir() {
        let cfg = DaemonConfig::from_toml_str(
            r#"
            [log]
            backend = "file"
            dir = "/var/lib/cdz/log"
            "#,
        )
        .unwrap();
        assert_eq!(
            cfg.log,
            LogConfig::File {
                dir: "/var/lib/cdz/log".into()
            }
        );
    }

    #[test]
    fn an_unknown_key_is_rejected_not_ignored() {
        // deny_unknown_fields: a typo'd key must be a loud error, not a silently-dropped setting.
        let err = DaemonConfig::from_toml_str("typo_field = \"oops\"\n").unwrap_err();
        assert!(err.0.contains("could not parse daemon config"), "{err}");
        // the unknown key is named in the underlying toml error, distinguishing a schema error from a
        // syntax error.
        assert!(err.0.contains("typo_field"), "{err}");
    }

    #[test]
    fn observability_enabled_without_a_target_is_rejected() {
        // enabled=true but no [[observability.target]] → error (an enabled metrics layer with nowhere to
        // emit is a misconfig, not a silent no-op).
        let err = DaemonConfig::from_toml_str(
            r#"
            [observability]
            enabled = true
            "#,
        )
        .unwrap_err();
        assert!(err.0.contains("at least one"), "{err}");
    }

    #[test]
    fn observability_report_interval_parses_and_rejects_zero_when_enabled() {
        // Custom interval parses.
        let cfg = DaemonConfig::from_toml_str(
            r#"
            [observability]
            enabled = true
            report_interval_secs = 15
            [[observability.target]]
            kind = "statsd"
            endpoint = "127.0.0.1:8125"
            "#,
        )
        .expect("custom report interval parses");
        assert_eq!(cfg.observability.report_interval_secs, 15);

        // 0 when enabled is rejected (would spin a tight flush loop).
        let err = DaemonConfig::from_toml_str(
            r#"
            [observability]
            enabled = true
            report_interval_secs = 0
            [[observability.target]]
            kind = "statsd"
            endpoint = "127.0.0.1:8125"
            "#,
        )
        .unwrap_err();
        assert!(err.0.contains("report_interval_secs"), "{err}");

        // 0 when DISABLED is fine (not validated — nothing reports).
        let cfg = DaemonConfig::from_toml_str(
            r#"
            [observability]
            enabled = false
            report_interval_secs = 0
            "#,
        )
        .expect("disabled skips interval validation");
        assert_eq!(cfg.observability.report_interval_secs, 0);
    }

    #[test]
    fn observability_fans_out_to_multiple_targets() {
        // The operator's config-MULTIPLE requirement: define MORE THAN ONE target backend, all parsed.
        let cfg = DaemonConfig::from_toml_str(
            r#"
            [observability]
            enabled = true
            [[observability.target]]
            kind = "statsd"
            endpoint = "10.0.0.1:9000"
            [[observability.target]]
            kind = "statsd"
            endpoint = "10.0.0.2:9000"
            prefix = "cdz.agent"
            "#,
        )
        .expect("multi-target observability parses");
        assert_eq!(cfg.observability.targets.len(), 2);
        assert_eq!(
            cfg.observability.targets[1],
            MetricsTarget::Statsd {
                endpoint: "10.0.0.2:9000".into(),
                prefix: Some("cdz.agent".into()),
            }
        );
    }

    #[test]
    fn observability_otlp_target_parses_with_and_without_optional_scopes() {
        // The otlp PUSH backend: endpoint required, scope_name/scope_version optional (both default None so
        // a minimal otlp target is just an endpoint).
        let cfg = DaemonConfig::from_toml_str(
            r#"
            [observability]
            enabled = true
            [[observability.target]]
            kind = "otlp"
            endpoint = "10.0.0.9:4317"
            [[observability.target]]
            kind = "otlp"
            endpoint = "10.0.0.10:4317"
            scope_name = "cdz-agent-host"
            scope_version = "0.1"
            "#,
        )
        .expect("otlp targets parse");
        assert_eq!(
            cfg.observability.targets[0],
            MetricsTarget::Otlp {
                endpoint: "10.0.0.9:4317".into(),
                scope_name: None,
                scope_version: None,
            }
        );
        assert_eq!(
            cfg.observability.targets[1],
            MetricsTarget::Otlp {
                endpoint: "10.0.0.10:4317".into(),
                scope_name: Some("cdz-agent-host".into()),
                scope_version: Some("0.1".into()),
            }
        );
        assert_eq!(cfg.observability.targets[1].kind(), "otlp");
    }

    #[test]
    fn observability_cloudwatch_target_parses_and_validates_namespace() {
        // The cloudwatch PUSH backend: namespace required (no endpoint — region/creds come from the SDK env).
        let cfg = DaemonConfig::from_toml_str(
            r#"
            [observability]
            enabled = true
            [[observability.target]]
            kind = "cloudwatch"
            namespace = "CDZ/AgentHost"
            "#,
        )
        .expect("cloudwatch target parses");
        assert_eq!(cfg.observability.targets.len(), 1);
        assert_eq!(
            cfg.observability.targets[0],
            MetricsTarget::CloudWatch {
                namespace: "CDZ/AgentHost".into(),
            }
        );
        assert_eq!(cfg.observability.targets[0].kind(), "cloudwatch");
    }

    #[test]
    fn observability_cloudwatch_empty_namespace_is_rejected() {
        // A present-but-blank namespace exercises the trim-check validation.
        let err = DaemonConfig::from_toml_str(
            r#"
            [observability]
            enabled = true
            [[observability.target]]
            kind = "cloudwatch"
            namespace = ""
            "#,
        )
        .unwrap_err();
        assert!(err.0.contains("namespace"), "{err}");
        assert!(err.0.contains("cloudwatch"), "names the kind: {err}");
    }

    #[test]
    fn observability_fans_out_across_backend_kinds() {
        // "Support all of them" + fan-out: a statsd AND an otlp target in one config, both parsed — the
        // daemon emits to both (mixed push backends).
        let cfg = DaemonConfig::from_toml_str(
            r#"
            [observability]
            enabled = true
            [[observability.target]]
            kind = "statsd"
            endpoint = "10.0.0.1:8125"
            [[observability.target]]
            kind = "otlp"
            endpoint = "10.0.0.2:4317"
            "#,
        )
        .expect("mixed-kind fan-out parses");
        assert_eq!(cfg.observability.targets.len(), 2);
        assert_eq!(cfg.observability.targets[0].kind(), "statsd");
        assert_eq!(cfg.observability.targets[1].kind(), "otlp");
    }

    #[test]
    fn observability_prometheus_loopback_bind_parses() {
        // A loopback prometheus bind just works — no allow_non_loopback needed.
        let cfg = DaemonConfig::from_toml_str(
            r#"
            [observability]
            enabled = true
            [[observability.target]]
            kind = "prometheus"
            bind = "127.0.0.1:9090"
            "#,
        )
        .expect("loopback prometheus bind parses");
        assert_eq!(cfg.observability.targets.len(), 1);
        assert_eq!(cfg.observability.targets[0].kind(), "prometheus");
    }

    #[test]
    fn observability_prometheus_empty_bind_is_rejected() {
        let err = DaemonConfig::from_toml_str(
            r#"
            [observability]
            enabled = true
            [[observability.target]]
            kind = "prometheus"
            bind = ""
            "#,
        )
        .unwrap_err();
        assert!(err.0.contains("bind"), "{err}");
        assert!(err.0.contains("prometheus"), "names the kind: {err}");
    }

    #[test]
    fn observability_prometheus_non_loopback_bind_is_rejected_without_opt_in() {
        // A non-loopback bind for the UNAUTHENTICATED scrape endpoint is rejected unless allow_non_loopback.
        let err = DaemonConfig::from_toml_str(
            r#"
            [observability]
            enabled = true
            [[observability.target]]
            kind = "prometheus"
            bind = "0.0.0.0:9090"
            "#,
        )
        .unwrap_err();
        assert!(
            err.0.contains("allow_non_loopback"),
            "points at the opt-in: {err}"
        );
    }

    #[test]
    fn observability_prometheus_non_loopback_bind_ok_with_opt_in() {
        // With the explicit opt-in, a non-loopback bind validates (still warned at boot at runtime).
        let cfg = DaemonConfig::from_toml_str(
            r#"
            [observability]
            enabled = true
            [[observability.target]]
            kind = "prometheus"
            bind = "0.0.0.0:9090"
            allow_non_loopback = true
            "#,
        )
        .expect("non-loopback bind with the opt-in validates");
        assert_eq!(cfg.observability.targets[0].kind(), "prometheus");
    }

    #[test]
    fn bind_is_loopback_requires_all_resolved_addrs_loopback() {
        // Literal loopback addresses classify loopback.
        assert!(bind_is_loopback("127.0.0.1:9090"));
        assert!(bind_is_loopback("[::1]:9090"));
        // A non-loopback literal is not loopback.
        assert!(!bind_is_loopback("0.0.0.0:9090"));
        assert!(!bind_is_loopback("10.0.0.5:9090"));
        // Fail-closed: unparseable/unresolvable → not loopback (needs the explicit opt-in).
        assert!(!bind_is_loopback("not-a-bind-addr"));
        assert!(!bind_is_loopback(""));
        // The #2160-review invariant: the check is ALL-addrs, not first-addr. `localhost` typically resolves
        // to both ::1 and 127.0.0.1 (all loopback) → loopback; but if it resolves to a non-loopback addr too,
        // it must be classified non-loopback. We can only assert the all-loopback direction deterministically
        // here (localhost → all loopback on a normal host); skip if localhost doesn't resolve.
        use std::net::ToSocketAddrs;
        if let Ok(addrs) = "localhost:9090".to_socket_addrs() {
            let all_lo = {
                let v: Vec<_> = addrs.collect();
                !v.is_empty() && v.iter().all(|a| a.ip().is_loopback())
            };
            // Our classifier must agree with "all resolved are loopback".
            assert_eq!(bind_is_loopback("localhost:9090"), all_lo);
        }
    }

    #[test]
    fn observability_otlp_empty_endpoint_is_rejected() {
        // The push-backend endpoint validation applies to otlp too (the or-pattern arm).
        let err = DaemonConfig::from_toml_str(
            r#"
            [observability]
            enabled = true
            [[observability.target]]
            kind = "otlp"
            endpoint = ""
            "#,
        )
        .unwrap_err();
        assert!(err.0.contains("endpoint"), "{err}");
        assert!(err.0.contains("otlp"), "names the offending kind: {err}");
    }

    #[test]
    fn observability_target_empty_endpoint_is_rejected() {
        // An endpoint that is PRESENT but blank — exercises the `endpoint.trim().is_empty()` validation.
        let err = DaemonConfig::from_toml_str(
            r#"
            [observability]
            enabled = true
            [[observability.target]]
            kind = "statsd"
            endpoint = ""
            "#,
        )
        .unwrap_err();
        assert!(err.0.contains("endpoint"), "{err}");
    }

    #[test]
    fn observability_target_omitted_endpoint_is_rejected() {
        // A DIFFERENT path from the empty case: the `statsd` variant's `endpoint` is required, so omitting
        // the field is a serde/TOML missing-field deser error — caught before the trim-check ever runs.
        // (This is a per-KIND typed field now, not a generic column — the whole point of the refine.)
        let err = DaemonConfig::from_toml_str(
            r#"
            [observability]
            enabled = true
            [[observability.target]]
            kind = "statsd"
            "#,
        )
        .unwrap_err();
        assert!(err.0.contains("endpoint"), "{err}");
    }

    #[test]
    fn observability_target_unknown_kind_is_rejected() {
        // Per-kind TYPED union (operator directive): an unrecognized `kind` is a loud deser error, not a
        // silently-accepted generic string. This is what the {kind, endpoint}→tagged-enum refine buys.
        let err = DaemonConfig::from_toml_str(
            r#"
            [observability]
            enabled = true
            [[observability.target]]
            kind = "not-a-real-backend"
            endpoint = "127.0.0.1:9000"
            "#,
        )
        .unwrap_err();
        // serde names the offending variant tag in an "unknown variant" error.
        assert!(err.0.contains("could not parse daemon config"), "{err}");
        assert!(err.0.contains("not-a-real-backend"), "{err}");
    }

    #[test]
    fn observability_target_foreign_field_is_rejected() {
        // A field that isn't part of the selected kind's typed config is rejected (deny_unknown_fields on the
        // variant) — a generic-string schema would have silently ignored it; the typed union catches it.
        let err = DaemonConfig::from_toml_str(
            r#"
            [observability]
            enabled = true
            [[observability.target]]
            kind = "statsd"
            endpoint = "127.0.0.1:9000"
            not_a_field = "oops"
            "#,
        )
        .unwrap_err();
        assert!(err.0.contains("could not parse daemon config"), "{err}");
        assert!(err.0.contains("not_a_field"), "{err}");
    }

    #[test]
    fn an_empty_dynamo_table_is_rejected() {
        let err = DaemonConfig::from_toml_str(
            r#"
            [log]
            backend = "dynamo"
            table = ""
            "#,
        )
        .unwrap_err();
        assert!(err.0.contains("table"), "{err}");
    }

    #[test]
    fn admin_defaults_to_disabled_and_carries_its_socket_when_set() {
        // Default: no [admin] section → disabled, no socket.
        let cfg = DaemonConfig::from_toml_str("").unwrap();
        assert_eq!(cfg.admin, AdminConfig::default());
        assert!(!cfg.admin.enabled);
        assert!(cfg.admin.socket.is_none());

        // Set: enabled + a socket path is carried through.
        let cfg = DaemonConfig::from_toml_str(
            r#"
            [admin]
            enabled = true
            socket = "/run/cdz/admin.sock"
            "#,
        )
        .expect("admin config parses");
        assert!(cfg.admin.enabled);
        assert_eq!(cfg.admin.socket.as_deref(), Some("/run/cdz/admin.sock"));
    }

    #[test]
    fn admin_enabled_without_a_socket_is_rejected() {
        let err = DaemonConfig::from_toml_str(
            r#"
            [admin]
            enabled = true
            "#,
        )
        .unwrap_err();
        assert!(err.0.contains("socket"), "{err}");
    }

    #[test]
    fn blob_defaults_to_memory_and_selects_a_dir_backend() {
        // Default: no [blob] section → in-memory.
        let cfg = DaemonConfig::from_toml_str("").unwrap();
        assert_eq!(cfg.blob, BlobConfig::Memory);

        // A dir backend carries its path.
        let cfg = DaemonConfig::from_toml_str(
            r#"
            [blob]
            backend = "dir"
            dir = "/var/lib/cdz/reducers"
            "#,
        )
        .expect("blob dir config parses");
        assert_eq!(
            cfg.blob,
            BlobConfig::Dir {
                dir: "/var/lib/cdz/reducers".into()
            }
        );
    }

    #[test]
    fn blob_cache_defaults_to_128mib_mem_2gib_disk_and_is_overridable() {
        // Default: no [blob_cache] section → 128 MiB in-memory, 2 GiB disk (concierge-confirmed defaults).
        let cfg = DaemonConfig::from_toml_str("").unwrap();
        assert_eq!(cfg.blob_cache.mem_bytes, 128 * 1024 * 1024);
        assert_eq!(cfg.blob_cache.disk_bytes, 2 * 1024 * 1024 * 1024);

        // Default: no disk_dir → disk tier off regardless of disk_bytes.
        assert_eq!(cfg.blob_cache.disk_dir, None, "no disk_dir by default");

        // Overridable per tier; 0 disables a tier; disk_dir opts the disk tier in.
        let cfg = DaemonConfig::from_toml_str(
            r#"
            [blob_cache]
            mem_bytes = 0
            disk_bytes = 1048576
            disk_dir = "/var/cache/cdz/blobs"
            "#,
        )
        .expect("blob_cache config parses");
        assert_eq!(cfg.blob_cache.mem_bytes, 0, "0 disables the in-memory tier");
        assert_eq!(cfg.blob_cache.disk_bytes, 1048576);
        assert_eq!(
            cfg.blob_cache.disk_dir.as_deref(),
            Some("/var/cache/cdz/blobs"),
            "disk_dir opts the on-disk tier in"
        );
    }

    #[test]
    fn an_empty_blob_dir_is_rejected() {
        let err = DaemonConfig::from_toml_str(
            r#"
            [blob]
            backend = "dir"
            dir = ""
            "#,
        )
        .unwrap_err();
        assert!(err.0.contains("dir"), "{err}");
    }

    #[test]
    fn an_s3_blob_backend_parses_with_bucket_and_optional_prefix() {
        // Bucket + explicit prefix.
        let cfg = DaemonConfig::from_toml_str(
            r#"
            [blob]
            backend = "s3"
            bucket = "cdz-reducers"
            prefix = "reducers/"
            "#,
        )
        .expect("blob s3 config parses");
        assert_eq!(
            cfg.blob,
            BlobConfig::S3 {
                bucket: "cdz-reducers".into(),
                prefix: "reducers/".into()
            }
        );
        // Prefix is optional → defaults to empty (bucket root).
        let cfg = DaemonConfig::from_toml_str(
            r#"
            [blob]
            backend = "s3"
            bucket = "cdz-reducers"
            "#,
        )
        .expect("blob s3 config parses without a prefix");
        assert_eq!(
            cfg.blob,
            BlobConfig::S3 {
                bucket: "cdz-reducers".into(),
                prefix: String::new()
            }
        );
    }

    #[test]
    fn an_empty_s3_bucket_is_rejected() {
        let err = DaemonConfig::from_toml_str(
            r#"
            [blob]
            backend = "s3"
            bucket = ""
            "#,
        )
        .unwrap_err();
        assert!(err.0.contains("bucket"), "{err}");
    }

    #[test]
    fn name_store_defaults_to_memory_and_selects_an_s3_backend() {
        // Default: no [name_store] section → memory (no durability, share-less — today's behavior).
        let cfg = DaemonConfig::from_toml_str("").unwrap();
        assert_eq!(cfg.name_store, NameStoreConfig::Memory);

        // An s3 backend carries its bucket + optional prefix (mirrors BlobConfig::S3).
        let cfg = DaemonConfig::from_toml_str(
            r#"
            [name_store]
            backend = "s3"
            bucket = "cdz-names"
            prefix = "names/"
            "#,
        )
        .expect("name_store s3 config parses");
        assert_eq!(
            cfg.name_store,
            NameStoreConfig::S3 {
                bucket: "cdz-names".into(),
                prefix: "names/".into()
            }
        );
        // Prefix is optional → defaults to empty (bucket root).
        let cfg = DaemonConfig::from_toml_str(
            r#"
            [name_store]
            backend = "s3"
            bucket = "cdz-names"
            "#,
        )
        .expect("name_store s3 config parses without a prefix");
        assert_eq!(
            cfg.name_store,
            NameStoreConfig::S3 {
                bucket: "cdz-names".into(),
                prefix: String::new()
            }
        );
    }

    #[test]
    fn an_empty_name_store_s3_bucket_is_rejected() {
        let err = DaemonConfig::from_toml_str(
            r#"
            [name_store]
            backend = "s3"
            bucket = ""
            "#,
        )
        .unwrap_err();
        assert!(err.0.contains("bucket"), "{err}");
        assert!(err.0.contains("name_store"), "names the section: {err}");
    }

    #[test]
    fn retry_should_retry_respects_max_attempts() {
        let r = RetryConfig {
            max_attempts: 3,
            base_ms: 100,
            max_ms: 30_000,
        };
        assert!(!r.should_retry(0), "attempt 0 is not a retry");
        assert!(r.should_retry(1), "1st retry allowed");
        assert!(r.should_retry(3), "3rd retry allowed (== max)");
        assert!(!r.should_retry(4), "4th retry denied (> max)");
        // max_attempts = 0 → never retry.
        let none = RetryConfig {
            max_attempts: 0,
            base_ms: 100,
            max_ms: 30_000,
        };
        assert!(!none.should_retry(1), "max_attempts=0 never retries");
    }

    #[test]
    fn retry_backoff_is_bounded_exponential() {
        let r = RetryConfig {
            max_attempts: 10,
            base_ms: 100,
            max_ms: 30_000,
        };
        assert_eq!(r.backoff_ms(0), 0, "no pre-first-retry delay");
        assert_eq!(r.backoff_ms(1), 100, "1st retry = base");
        assert_eq!(r.backoff_ms(2), 200, "doubles");
        assert_eq!(r.backoff_ms(3), 400);
        assert_eq!(r.backoff_ms(4), 800);
        // Saturates at max_ms (100 * 2^9 = 51200 > 30000 → capped).
        assert_eq!(r.backoff_ms(10), 30_000, "capped at max_ms");
        // A large attempt can't overflow — pins at max_ms (saturating shift + mul + min).
        assert_eq!(
            r.backoff_ms(1000),
            30_000,
            "huge attempt saturates, no panic/overflow"
        );
    }

    #[test]
    fn tracing_config_parses_all_fields_and_file_output() {
        let cfg = DaemonConfig::from_toml_str(
            r#"
            [tracing]
            enabled = true
            filter = "cdz_agent_host=debug,info"
            format = "json"
            [tracing.output]
            to = "file"
            path = "/var/log/cdz/trace.log"
            "#,
        )
        .expect("full tracing config parses");
        assert!(cfg.tracing.enabled);
        assert_eq!(cfg.tracing.filter, "cdz_agent_host=debug,info");
        assert_eq!(cfg.tracing.format, TracingFormat::Json);
        assert_eq!(
            cfg.tracing.output,
            TracingOutput::File {
                path: "/var/log/cdz/trace.log".into()
            }
        );
    }

    #[test]
    fn tracing_enabled_with_a_blank_filter_is_rejected() {
        let err = DaemonConfig::from_toml_str(
            r#"
            [tracing]
            enabled = true
            filter = ""
            "#,
        )
        .unwrap_err();
        assert!(err.0.contains("filter"), "{err}");
    }

    #[test]
    fn tracing_file_output_needs_a_path() {
        // A file output with a blank path is a misconfig — caught at validate (serde requires the field, so
        // this exercises the trim-check on a present-but-blank value).
        let err = DaemonConfig::from_toml_str(
            r#"
            [tracing]
            enabled = true
            [tracing.output]
            to = "file"
            path = ""
            "#,
        )
        .unwrap_err();
        assert!(err.0.contains("path"), "{err}");
    }

    #[test]
    fn tracing_disabled_skips_validation_of_a_would_be_invalid_filter() {
        // When disabled, validate() skips the filter/output checks (no subscriber installed). Prove it by
        // giving a filter that WOULD be rejected if enabled (blank — see
        // tracing_enabled_with_a_blank_filter_is_rejected): with enabled=false it still parses + validates OK,
        // so this proves disabled genuinely skips the check (a validate-when-disabled bug would fail here).
        let cfg = DaemonConfig::from_toml_str(
            r#"
            [tracing]
            enabled = false
            filter = ""
            "#,
        )
        .expect("disabled tracing skips filter validation");
        assert!(!cfg.tracing.enabled);
        assert_eq!(cfg.tracing.filter, "");
    }
}
