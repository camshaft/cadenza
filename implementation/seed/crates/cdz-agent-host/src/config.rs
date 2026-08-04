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
/// `camshaft/s2n-quic` branch `main`). Disabled by default; when enabled, metrics FAN OUT to the configured
/// `targets` — a LIST so MULTIPLE backends are supported (the operator's config-multiple requirement), with
/// at least one required when enabled (validated below). Each `[[observability.target]]` is a
/// [`MetricsTarget`] whose `kind` selects a backend and whose OTHER fields are that backend's OWN typed
/// config, so the daemon can emit to several sinks (multiple statsd collectors, or different backend types).
///
/// The emitter itself is a following slice (feature-gated to keep the QUIC/metrics dep tree out of the
/// default build), waiting on the operator's confirmation of the v0 backend SET. This CONFIG is always
/// parseable ahead of that wiring (staged like `[log]`/`[blob]`: config first, emitter follows).
#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct ObservabilityConfig {
    /// Emit metrics. Default off. When true, `targets` must be non-empty (validated below).
    #[serde(default)]
    pub enabled: bool,
    /// The metrics target backends to FAN OUT to — a LIST (not a single endpoint) so metrics can go to
    /// several sinks (the operator's config-MULTIPLE requirement). TOML: `[[observability.target]]` tables.
    #[serde(default, rename = "target")]
    pub targets: Vec<MetricsTarget>,
}

/// One metrics target backend — a per-kind TYPED config, NOT a generic `{kind, endpoint}` pair (operator
/// directive: "each backend needs its own config fields - not just some generic target string"). `kind`
/// discriminates the variant (a tagged union, like [`LogConfig`]/[`BlobConfig`]); each variant carries the
/// fields THAT backend needs, validated per-kind. Adding a backend adds a typed variant with its own fields,
/// so a mis-shaped target (a field that belongs to another backend, a missing required field) is a loud
/// deser error rather than an ignored generic string.
///
/// The variants model REAL `s2n-quic-dc-metrics` backends. The operator's direction is "support all of
/// them" (Statsd/Otlp/Prometheus/Querylog/Arrow). They fall into three CONFIG SHAPES — this enum grows one
/// variant per backend as each shape is confirmed:
/// - PUSH (a caller-supplied sink the daemon periodically flushes to): `statsd` (UDP) + `otlp` (OTLP
///   payloads). Modeled here — an `endpoint` the daemon sends to.
/// - PULL (the backend exposes state an external scraper reads): `prometheus` — the daemon would run a
///   scrape server; its config is a bind address, a distinct shape (pending operator confirm on running an
///   inbound listener).
/// - FILE (no network; output the daemon writes locally): `querylog` / `arrow` (Parquet) — config is a
///   path/rotation, another distinct shape (pending operator confirm on path + rotation).
///
/// The push variants land first (unambiguous — same sink shape); prometheus/querylog/arrow are held pending
/// the config-shape confirm (see the concierge scoping ask), each a cheap additive variant when confirmed.
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
    /// arguments); both default so a minimal config is just an endpoint.
    Otlp {
        /// The OTLP collector endpoint the sink sends to (`host:port` or a URL, per the sink transport).
        endpoint: String,
        /// The OTLP instrumentation-scope name embedded in every metric (defaults to the crate name).
        #[serde(default)]
        scope_name: Option<String>,
        /// The OTLP instrumentation-scope version (defaults to unset).
        #[serde(default)]
        scope_version: Option<String>,
    },
}

impl MetricsTarget {
    /// The backend kind token (matches the TOML `kind = "…"` tag) — for diagnostics + fan-out logging.
    pub fn kind(&self) -> &'static str {
        match self {
            MetricsTarget::Statsd { .. } => "statsd",
            MetricsTarget::Otlp { .. } => "otlp",
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
/// **Parsed-but-not-yet-wired (staging gap, #1981 review).** Like `[log]`/`[observability]`, this config
/// SECTION lands ahead of the code that reads it: the daemon bin does not yet consult `config.blob` (the
/// factory-wiring slice — [`ComponentSessionFactory`](crate::ComponentSessionFactory) + a config-built blob
/// store — is the follow-up). So a `backend = "dir"` currently PARSES + VALIDATES but selects no store yet;
/// it takes effect once that slice lands. Documented so the no-op is a conscious staging gap, not a silent
/// surprise (the "config that does nothing" hazard).
#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
#[serde(tag = "backend", rename_all = "kebab-case", deny_unknown_fields)]
pub enum BlobConfig {
    /// In-memory blob store — non-durable, dev/test (an install only finds a reducer `put` this run). Default.
    #[default]
    Memory,
    /// Disk-backed content-addressed store rooted at `dir` — the single-host durable reducer store.
    Dir { dir: String },
}

/// Effect-retry policy: a bounded exponential backoff the daemon's supervisor applies to a RETRYABLE
/// effect outcome (a transient Bedrock/HTTP failure classified `RETRYABLE:` by [`crate::retry`]). A
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
        assert_eq!(cfg.retries, RetryConfig::default());
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
}
