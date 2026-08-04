//! The agent-daemon CONFIG — one TOML file that configures the whole daemon (operator directive: "set the
//! entire thing up via config and not from cli args; the only arg I want is the path to the config").
//!
//! [`DaemonConfig`] is the root: parse it from a TOML path with [`DaemonConfig::from_toml_path`], and every
//! later daemon concern reads its section — the durable-log backend (`[log]`, file dev / Dynamo prod,
//! config-SELECTABLE per the generic-async-log directive), observability (`[observability]`, the operator's
//! s2n-quic-dc-metrics), effect-retry policy (`[retries]`), and the sessions to spawn (`[[session]]`). This
//! module is the config SPINE — schema + parse + validation; the wiring of each backend to real code lands
//! as its own daemon slice. Unknown fields are REJECTED (`deny_unknown_fields`) so a typo'd key is a loud
//! config error, not a silently-ignored setting.

use serde::Deserialize;
use std::path::Path;

/// The whole daemon configuration, deserialized from one TOML file. Sections default when omitted so a
/// minimal config (just `[[session]]` entries) boots with sensible dev defaults (in-memory log, metrics
/// off, default retry policy).
#[derive(Debug, Clone, Deserialize, PartialEq)]
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
    /// The sessions this daemon hosts. At least one is required (a daemon with no sessions does nothing) —
    /// parsing via [`from_toml_str`](DaemonConfig::from_toml_str) enforces that.
    #[serde(default)]
    pub session: Vec<SessionConfig>,
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

/// Observability config — the operator's s2n-quic-dc-metrics integration. Disabled by default; when
/// enabled, `endpoint` is where metrics are emitted (the wiring lands as the observability daemon slice).
#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct ObservabilityConfig {
    /// Emit metrics via s2n-quic-dc-metrics. Default off.
    #[serde(default)]
    pub enabled: bool,
    /// Where to emit metrics to (a collector address); required-ish when `enabled` — validated below.
    #[serde(default)]
    pub endpoint: Option<String>,
}

/// Effect-retry policy: a bounded exponential backoff the daemon's supervisor applies to a RETRYABLE
/// effect outcome (a transient Bedrock/HTTP failure classified `RETRYABLE:` by [`crate::retry`]). A
/// PERMANENT failure is never retried. (The retry MECHANISM — re-emit via Timer — is a userspace/supervisor
/// concern; this is the POLICY the daemon's driver reads.)
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

/// One hosted session the daemon spawns at boot: an `id` (its registry key) and the `reducer` component to
/// run it (a path to a wasm reducer blob — the daemon loads + runs it). Policy is referenced by name (the
/// §20b `POLICY_CURRENT` pointer) rather than inlined here, so a session's authorizer is swappable at
/// runtime; a `policy` path here seeds the initial policy component.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SessionConfig {
    /// The session's registry id.
    pub id: String,
    /// Path to the session's reducer wasm component.
    pub reducer: String,
    /// Optional path to the initial Cedar policy component (else the session starts deny-all until a
    /// POLICY_CURRENT store/set swaps one in).
    #[serde(default)]
    pub policy: Option<String>,
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
            toml::from_str(text).map_err(|e| ConfigError(format!("invalid TOML: {e}")))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Semantic validation beyond what the type system + serde enforce: at least one session, non-empty
    /// backend paths, and observability endpoint present when enabled.
    fn validate(&self) -> Result<(), ConfigError> {
        if self.session.is_empty() {
            return Err(ConfigError(
                "no [[session]] configured — a daemon with no sessions does nothing".into(),
            ));
        }
        for s in &self.session {
            if s.id.trim().is_empty() {
                return Err(ConfigError("a [[session]] has an empty id".into()));
            }
            if s.reducer.trim().is_empty() {
                return Err(ConfigError(format!(
                    "session {:?} has an empty reducer path",
                    s.id
                )));
            }
        }
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
        if self.observability.enabled && self.observability.endpoint.is_none() {
            return Err(ConfigError(
                "[observability] enabled=true needs an endpoint".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minimal_config_boots_with_defaults() {
        let cfg = DaemonConfig::from_toml_str(
            r#"
            [[session]]
            id = "worker-1"
            reducer = "/blobs/worker.wasm"
            "#,
        )
        .expect("minimal config parses");
        // Omitted sections take their defaults.
        assert_eq!(cfg.log, LogConfig::Memory);
        assert!(!cfg.observability.enabled);
        assert_eq!(cfg.retries, RetryConfig::default());
        assert_eq!(cfg.session.len(), 1);
        assert_eq!(cfg.session[0].id, "worker-1");
        assert_eq!(cfg.session[0].policy, None);
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
            endpoint = "127.0.0.1:9000"

            [retries]
            max_attempts = 5
            base_ms = 200
            max_ms = 10000

            [[session]]
            id = "a"
            reducer = "/blobs/a.wasm"
            policy = "/blobs/policy.wasm"

            [[session]]
            id = "b"
            reducer = "/blobs/b.wasm"
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
        assert_eq!(
            cfg.observability.endpoint.as_deref(),
            Some("127.0.0.1:9000")
        );
        assert_eq!(cfg.retries.max_attempts, 5);
        assert_eq!(cfg.session.len(), 2);
        assert_eq!(cfg.session[0].policy.as_deref(), Some("/blobs/policy.wasm"));
    }

    #[test]
    fn a_file_log_backend_carries_its_dir() {
        let cfg = DaemonConfig::from_toml_str(
            r#"
            [log]
            backend = "file"
            dir = "/var/lib/cdz/log"
            [[session]]
            id = "x"
            reducer = "/x.wasm"
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
    fn no_sessions_is_a_validation_error() {
        let err = DaemonConfig::from_toml_str("[log]\nbackend = \"memory\"\n").unwrap_err();
        assert!(err.0.contains("no [[session]]"), "{err}");
    }

    #[test]
    fn an_unknown_key_is_rejected_not_ignored() {
        // deny_unknown_fields: a typo'd key must be a loud error, not a silently-dropped setting.
        let err = DaemonConfig::from_toml_str(
            r#"
            [[session]]
            id = "x"
            reducer = "/x.wasm"
            typo_field = "oops"
            "#,
        )
        .unwrap_err();
        assert!(err.0.contains("invalid TOML"), "{err}");
    }

    #[test]
    fn observability_enabled_without_endpoint_is_rejected() {
        let err = DaemonConfig::from_toml_str(
            r#"
            [observability]
            enabled = true
            [[session]]
            id = "x"
            reducer = "/x.wasm"
            "#,
        )
        .unwrap_err();
        assert!(err.0.contains("endpoint"), "{err}");
    }

    #[test]
    fn an_empty_dynamo_table_is_rejected() {
        let err = DaemonConfig::from_toml_str(
            r#"
            [log]
            backend = "dynamo"
            table = ""
            [[session]]
            id = "x"
            reducer = "/x.wasm"
            "#,
        )
        .unwrap_err();
        assert!(err.0.contains("table"), "{err}");
    }
}
