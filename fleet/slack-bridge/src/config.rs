//! Bridge configuration — Slack credentials + fleet wiring, loaded fail-soft.
//!
//! The assign requires the bridge to **fail soft when tokens are absent** (log + carry on, never crash),
//! so it can be built, land, and run before the operator has created the Slack app. This module resolves
//! config from, in priority order:
//!   1. environment variables (`SLACK_BOT_TOKEN`, `SLACK_APP_TOKEN`, …),
//!   2. a gitignored TOML file (`.claude/fleet/slack.toml` by default, or `$SLACK_BRIDGE_CONFIG`),
//!   3. built-in defaults for the non-secret fields.
//!
//! Credentials are NEVER hardcoded or committed — `slack.toml` is gitignored (see `.gitignore`). A
//! [`Config`] whose [`Config::tokens`] returns `None` is valid: the caller logs "tokens absent, idle" and
//! the transport loop stays dormant, retrying, rather than panicking.

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// The two Slack credentials the Socket Mode client needs. Present together or not at all — a bridge with
/// only one token can't run, so [`Config::tokens`] yields `Some` only when BOTH are set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackTokens {
    /// Bot User OAuth Token (`xoxb-…`) — used for `chat.postMessage` etc.
    pub bot_token: String,
    /// App-Level Token (`xapp-…`, scope `connections:write`) — enables Socket Mode.
    pub app_token: String,
}

/// Fully-resolved bridge configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Bot token, if provided (env or file). `None` = fail-soft dormant mode.
    pub bot_token: Option<String>,
    /// App-level token, if provided.
    pub app_token: Option<String>,
    /// The Slack channel ID the bridge posts fleet→operator messages into (e.g. `C0123ABCD`). Optional:
    /// without it the bridge is inbound-only (DMs) and can't mirror asks.
    pub channel: Option<String>,
    /// The fleet state dir holding `inbox/` (the SHARED runtime dir, e.g. `<repo>/.claude/fleet`).
    pub fleet_dir: PathBuf,
    /// Default recipient when the operator gives no `@agent` (the concierge).
    pub default_to: String,
    /// This bridge's own fleet agent name / inbox.
    pub bridge_agent: String,
}

/// The subset read from the optional TOML file. Every field optional; env overrides any of these.
#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    bot_token: Option<String>,
    app_token: Option<String>,
    channel: Option<String>,
    fleet_dir: Option<String>,
    default_to: Option<String>,
    bridge_agent: Option<String>,
}

const DEFAULT_DEFAULT_TO: &str = "concierge";
const DEFAULT_BRIDGE_AGENT: &str = "slack-bridge";

impl Config {
    /// The tokens if and only if BOTH are present — the precondition for starting the transport.
    pub fn tokens(&self) -> Option<SlackTokens> {
        match (&self.bot_token, &self.app_token) {
            (Some(b), Some(a)) if !b.is_empty() && !a.is_empty() => Some(SlackTokens {
                bot_token: b.clone(),
                app_token: a.clone(),
            }),
            _ => None,
        }
    }

    /// Resolve config from the process environment plus an optional TOML file, using `default_fleet_dir`
    /// when neither env nor file sets one. Pure w.r.t. its inputs: `env` is a lookup closure (so tests
    /// pass a fixed map instead of touching the real environment), and the file is read from disk only if
    /// present. Never fails on a missing file or missing tokens — that's the fail-soft contract.
    pub fn resolve<F>(env: F, default_fleet_dir: &Path) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        // Locate the config file: $SLACK_BRIDGE_CONFIG, else <fleet_dir candidate>/slack.toml. We need a
        // fleet-dir guess first (env or default) to find a relative slack.toml.
        let fleet_dir_guess = env("FLEET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| default_fleet_dir.to_path_buf());
        let file_path = env("SLACK_BRIDGE_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(|| fleet_dir_guess.join("slack.toml"));
        let file = read_file_config(&file_path);

        // env wins over file wins over default.
        let pick = |key: &str, file_val: Option<String>| env(key).or(file_val);

        let fleet_dir = pick("FLEET_DIR", file.fleet_dir)
            .map(PathBuf::from)
            .unwrap_or_else(|| default_fleet_dir.to_path_buf());

        Config {
            bot_token: pick("SLACK_BOT_TOKEN", file.bot_token).filter(|s| !s.is_empty()),
            app_token: pick("SLACK_APP_TOKEN", file.app_token).filter(|s| !s.is_empty()),
            channel: pick("SLACK_BRIDGE_CHANNEL", file.channel).filter(|s| !s.is_empty()),
            fleet_dir,
            default_to: pick("FLEET_DEFAULT_TO", file.default_to)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| DEFAULT_DEFAULT_TO.to_string()),
            bridge_agent: pick("SLACK_BRIDGE_AGENT", file.bridge_agent)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| DEFAULT_BRIDGE_AGENT.to_string()),
        }
    }

    /// Convenience: resolve from the REAL process environment.
    pub fn from_env(default_fleet_dir: &Path) -> Self {
        Self::resolve(|k| std::env::var(k).ok(), default_fleet_dir)
    }
}

/// Read + parse the TOML config file. A missing file, or one that fails to parse, yields defaults (empty)
/// rather than an error — fail-soft. A parse error is worth surfacing to the caller's log, so it returns
/// the parse outcome via `eprintln!` here (the bridge has no structured logger yet) and an empty config.
fn read_file_config(path: &Path) -> FileConfig {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return FileConfig::default(), // absent = fine
    };
    match toml::from_str::<FileConfig>(&text) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("slack-bridge: ignoring malformed {}: {e}", path.display());
            FileConfig::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn env_map(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k: &str| m.get(k).cloned()
    }

    fn tmp_dir(tag: &str) -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let d =
            std::env::temp_dir().join(format!("slack-cfg-{}-{}-{}", tag, std::process::id(), n));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn absent_everything_is_dormant_but_valid() {
        let dir = tmp_dir("empty");
        let cfg = Config::resolve(env_map(&[]), &dir);
        assert!(cfg.tokens().is_none(), "no tokens → dormant");
        assert_eq!(cfg.default_to, "concierge");
        assert_eq!(cfg.bridge_agent, "slack-bridge");
        assert_eq!(cfg.fleet_dir, dir);
    }

    #[test]
    fn only_one_token_is_still_dormant() {
        let dir = tmp_dir("onetok");
        let cfg = Config::resolve(env_map(&[("SLACK_BOT_TOKEN", "xoxb-1")]), &dir);
        assert!(cfg.tokens().is_none(), "one token is not enough to run");
        assert_eq!(cfg.bot_token.as_deref(), Some("xoxb-1"));
    }

    #[test]
    fn both_tokens_from_env_yield_tokens() {
        let dir = tmp_dir("bothtok");
        let cfg = Config::resolve(
            env_map(&[("SLACK_BOT_TOKEN", "xoxb-1"), ("SLACK_APP_TOKEN", "xapp-2")]),
            &dir,
        );
        let t = cfg.tokens().expect("both present");
        assert_eq!(t.bot_token, "xoxb-1");
        assert_eq!(t.app_token, "xapp-2");
    }

    #[test]
    fn env_overrides_and_defaults_apply() {
        let dir = tmp_dir("over");
        let cfg = Config::resolve(
            env_map(&[
                ("SLACK_BRIDGE_CHANNEL", "C123"),
                ("FLEET_DEFAULT_TO", "pr-sync"),
                ("SLACK_BRIDGE_AGENT", "sb2"),
            ]),
            &dir,
        );
        assert_eq!(cfg.channel.as_deref(), Some("C123"));
        assert_eq!(cfg.default_to, "pr-sync");
        assert_eq!(cfg.bridge_agent, "sb2");
    }

    #[test]
    fn reads_toml_file_and_env_wins() {
        let dir = tmp_dir("file");
        let cfg_path = dir.join("slack.toml");
        std::fs::write(
            &cfg_path,
            "bot_token = \"xoxb-file\"\napp_token = \"xapp-file\"\nchannel = \"Cfile\"\ndefault_to = \"design\"\n",
        )
        .unwrap();
        // FLEET_DIR points at `dir` so the file is found at dir/slack.toml; env overrides bot_token.
        let cfg = Config::resolve(
            env_map(&[
                ("FLEET_DIR", dir.to_str().unwrap()),
                ("SLACK_BOT_TOKEN", "xoxb-env"),
            ]),
            &dir,
        );
        assert_eq!(
            cfg.bot_token.as_deref(),
            Some("xoxb-env"),
            "env wins over file"
        );
        assert_eq!(
            cfg.app_token.as_deref(),
            Some("xapp-file"),
            "file fills the rest"
        );
        assert_eq!(cfg.channel.as_deref(), Some("Cfile"));
        assert_eq!(cfg.default_to, "design");
        assert!(cfg.tokens().is_some());
    }

    #[test]
    fn malformed_toml_is_ignored_not_fatal() {
        let dir = tmp_dir("bad");
        std::fs::write(dir.join("slack.toml"), "this is not = = valid toml [[[").unwrap();
        let cfg = Config::resolve(env_map(&[("FLEET_DIR", dir.to_str().unwrap())]), &dir);
        // Falls back to dormant defaults rather than crashing.
        assert!(cfg.tokens().is_none());
        assert_eq!(cfg.default_to, "concierge");
    }

    #[test]
    fn explicit_config_path_env_is_honored() {
        let dir = tmp_dir("explicit");
        let cfg_path = dir.join("custom.toml");
        std::fs::write(
            &cfg_path,
            "app_token = \"xapp-c\"\nbot_token = \"xoxb-c\"\n",
        )
        .unwrap();
        let cfg = Config::resolve(
            env_map(&[("SLACK_BRIDGE_CONFIG", cfg_path.to_str().unwrap())]),
            &dir,
        );
        assert!(
            cfg.tokens().is_some(),
            "tokens loaded from the explicit path"
        );
    }
}
