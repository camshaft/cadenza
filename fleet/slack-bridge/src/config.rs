//! Bridge configuration — Slack credentials + fleet wiring, loaded fail-soft.
//!
//! The assign requires the bridge to **fail soft when tokens are absent** (log + carry on, never crash),
//! so it can be built, land, and run before the operator has created the Slack app. This module resolves
//! config from, in priority order:
//!   1. environment variables (`SLACK_BOT_TOKEN`, `SLACK_APP_TOKEN`, …),
//!   2. `~/.cadenza-env` — a home-dir, out-of-repo dotenv file the operator drops their tokens in,
//!   3. a gitignored TOML file (`.claude/fleet/slack.toml` by default, or `$SLACK_BRIDGE_CONFIG`),
//!   4. built-in defaults for the non-secret fields.
//!
//! Credentials are NEVER hardcoded or committed — `slack.toml` is gitignored (see `.gitignore`). A
//! [`Config`] whose [`Config::tokens`] returns `None` is valid: the caller logs "tokens absent, idle" and
//! the transport loop stays dormant, retrying, rather than panicking.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// Redact a secret for `Debug`: keep only the `xoxb-`/`xapp-` style prefix so logs stay diagnosable
/// without ever printing the token body. SECURITY (PR #393): these structs hold live Slack credentials; a
/// stray `{:?}`/`dbg!`/panic-format must not leak them, so `Debug` is hand-rolled to redact.
fn redact(secret: &str) -> String {
    match secret.split_once('-') {
        Some((prefix, _)) if !prefix.is_empty() => format!("{prefix}-***"),
        _ if secret.is_empty() => "<unset>".to_string(),
        _ => "***".to_string(),
    }
}

fn redact_opt(secret: &Option<String>) -> String {
    match secret {
        Some(s) => redact(s),
        None => "<none>".to_string(),
    }
}

/// The two Slack credentials the Socket Mode client needs. Present together or not at all — a bridge with
/// only one token can't run, so [`Config::tokens`] yields `Some` only when BOTH are set.
///
/// NOTE: `Debug` is REDACTING (no derive) — see [`redact`]. PR #393 security hygiene.
#[derive(Clone, PartialEq, Eq)]
pub struct SlackTokens {
    /// Bot User OAuth Token (`xoxb-…`) — used for `chat.postMessage` etc.
    pub bot_token: String,
    /// App-Level Token (`xapp-…`, scope `connections:write`) — enables Socket Mode.
    pub app_token: String,
}

impl fmt::Debug for SlackTokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SlackTokens")
            .field("bot_token", &redact(&self.bot_token))
            .field("app_token", &redact(&self.app_token))
            .finish()
    }
}

/// Fully-resolved bridge configuration.
///
/// NOTE: `Debug` is REDACTING (no derive) so the token fields never print raw. PR #393 security hygiene.
#[derive(Clone, PartialEq, Eq)]
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

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("bot_token", &redact_opt(&self.bot_token))
            .field("app_token", &redact_opt(&self.app_token))
            .field("channel", &self.channel)
            .field("fleet_dir", &self.fleet_dir)
            .field("default_to", &self.default_to)
            .field("bridge_agent", &self.bridge_agent)
            .finish()
    }
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

    /// Resolve config from the process environment plus an optional TOML file (no `~/.cadenza-env` layer).
    /// Pure w.r.t. its inputs: `env` is a lookup closure (tests pass a fixed map). Kept for tests + as the
    /// thin base; [`Config::resolve_layered`] adds the dotenv layer. Fail-soft: missing file/tokens is fine.
    pub fn resolve<F>(env: F, default_fleet_dir: &Path) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        Self::resolve_layered(env, &BTreeMap::new(), default_fleet_dir)
    }

    /// Resolve config with a THREE-layer precedence: process env > `dotenv` map (`~/.cadenza-env`) >
    /// `slack.toml`. The operator drops their tokens in `~/.cadenza-env` (a home-dir, out-of-repo secret
    /// file), so it slots between explicit env vars and the repo-local toml. Pure w.r.t. inputs: both `env`
    /// and `dotenv` are supplied by the caller ([`Config::from_env`] reads the real ones). Fail-soft.
    pub fn resolve_layered<F>(
        env: F,
        dotenv: &BTreeMap<String, String>,
        default_fleet_dir: &Path,
    ) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        // Locate the toml config: $SLACK_BRIDGE_CONFIG, else <fleet_dir guess>/slack.toml.
        let fleet_dir_guess = env("FLEET_DIR")
            .or_else(|| dotenv.get("FLEET_DIR").cloned())
            .map(PathBuf::from)
            .unwrap_or_else(|| default_fleet_dir.to_path_buf());
        let file_path = env("SLACK_BRIDGE_CONFIG")
            .or_else(|| dotenv.get("SLACK_BRIDGE_CONFIG").cloned())
            .map(PathBuf::from)
            .unwrap_or_else(|| fleet_dir_guess.join("slack.toml"));
        let file = read_file_config(&file_path);

        // env > dotenv > file > default.
        let pick = |key: &str, file_val: Option<String>| {
            env(key).or_else(|| dotenv.get(key).cloned()).or(file_val)
        };

        let fleet_dir = pick("FLEET_DIR", file.fleet_dir)
            .map(PathBuf::from)
            .unwrap_or_else(|| default_fleet_dir.to_path_buf());

        Config {
            bot_token: pick("SLACK_BOT_TOKEN", file.bot_token).filter(|s| !s.is_empty()),
            app_token: pick("SLACK_APP_TOKEN", file.app_token).filter(|s| !s.is_empty()),
            // Accept SLACK_BRIDGE_CHANNEL or the shorter SLACK_CHANNEL alias (the dotenv file may use either).
            channel: pick("SLACK_BRIDGE_CHANNEL", file.channel)
                .or_else(|| env("SLACK_CHANNEL").or_else(|| dotenv.get("SLACK_CHANNEL").cloned()))
                .filter(|s| !s.is_empty()),
            fleet_dir,
            default_to: pick("FLEET_DEFAULT_TO", file.default_to)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| DEFAULT_DEFAULT_TO.to_string()),
            bridge_agent: pick("SLACK_BRIDGE_AGENT", file.bridge_agent)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| DEFAULT_BRIDGE_AGENT.to_string()),
        }
    }

    /// Convenience: resolve from the REAL process environment + `~/.cadenza-env` (dotenv) if present.
    pub fn from_env(default_fleet_dir: &Path) -> Self {
        let dotenv = home_cadenza_env();
        Self::resolve_layered(|k| std::env::var(k).ok(), &dotenv, default_fleet_dir)
    }
}

/// Parse dotenv-style `KEY=VALUE` lines (as in `~/.cadenza-env`): ignores blank lines and `#` comments,
/// trims whitespace, strips one layer of surrounding quotes from the value, and honors a leading `export`.
/// Pure — unit-tested. Unknown keys are kept (the caller only reads the ones it knows).
pub fn parse_dotenv(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let key = k.trim().to_string();
        let mut val = v.trim();
        // Strip one layer of matching quotes.
        if (val.starts_with('"') && val.ends_with('"') && val.len() >= 2)
            || (val.starts_with('\'') && val.ends_with('\'') && val.len() >= 2)
        {
            val = &val[1..val.len() - 1];
        }
        if !key.is_empty() {
            out.insert(key, val.to_string());
        }
    }
    out
}

/// Read `~/.cadenza-env` into a map, or empty if absent/unreadable (fail-soft). `$HOME` missing → empty.
fn home_cadenza_env() -> BTreeMap<String, String> {
    let Some(home) = std::env::var_os("HOME") else {
        return BTreeMap::new();
    };
    let path = PathBuf::from(home).join(".cadenza-env");
    match std::fs::read_to_string(&path) {
        Ok(text) => parse_dotenv(&text),
        Err(_) => BTreeMap::new(),
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

    // ── ~/.cadenza-env dotenv layer + precedence ────────────────────────────────────────────────

    #[test]
    fn parse_dotenv_handles_comments_export_quotes() {
        let text = "# comment\n\nexport SLACK_BOT_TOKEN=xoxb-1\nSLACK_APP_TOKEN=\"xapp-2\"\nSLACK_CHANNEL='D0X'\nbad line no equals\n";
        let m = parse_dotenv(text);
        assert_eq!(m.get("SLACK_BOT_TOKEN").unwrap(), "xoxb-1");
        assert_eq!(
            m.get("SLACK_APP_TOKEN").unwrap(),
            "xapp-2",
            "double quotes stripped"
        );
        assert_eq!(
            m.get("SLACK_CHANNEL").unwrap(),
            "D0X",
            "single quotes stripped"
        );
        assert!(!m.contains_key("bad line no equals"));
    }

    #[test]
    fn dotenv_supplies_tokens_when_env_absent() {
        let dir = tmp_dir("dotenv");
        let dotenv =
            parse_dotenv("SLACK_BOT_TOKEN=xoxb-d\nSLACK_APP_TOKEN=xapp-d\nSLACK_CHANNEL=D0DM\n");
        let cfg = Config::resolve_layered(env_map(&[]), &dotenv, &dir);
        let t = cfg.tokens().expect("tokens from dotenv");
        assert_eq!(t.bot_token, "xoxb-d");
        assert_eq!(t.app_token, "xapp-d");
        assert_eq!(
            cfg.channel.as_deref(),
            Some("D0DM"),
            "SLACK_CHANNEL alias honored"
        );
    }

    #[test]
    fn precedence_env_over_dotenv_over_file() {
        let dir = tmp_dir("prec");
        std::fs::write(
            dir.join("slack.toml"),
            "bot_token = \"xoxb-file\"\napp_token = \"xapp-file\"\n",
        )
        .unwrap();
        let dotenv = parse_dotenv("SLACK_BOT_TOKEN=xoxb-dotenv\n");
        let cfg = Config::resolve_layered(
            env_map(&[
                ("FLEET_DIR", dir.to_str().unwrap()),
                ("SLACK_APP_TOKEN", "xapp-env"),
            ]),
            &dotenv,
            &dir,
        );
        assert_eq!(cfg.app_token.as_deref(), Some("xapp-env"), "env wins");
        assert_eq!(
            cfg.bot_token.as_deref(),
            Some("xoxb-dotenv"),
            "dotenv beats file"
        );
    }

    // ── SECURITY: redacting Debug (PR #393) ─────────────────────────────────────────────────────

    #[test]
    fn debug_redacts_secrets() {
        let t = SlackTokens {
            bot_token: "xoxb-SECRETBODY".into(),
            app_token: "xapp-SECRETBODY".into(),
        };
        let dbg = format!("{t:?}");
        assert!(
            !dbg.contains("SECRETBODY"),
            "token body must not appear: {dbg}"
        );
        assert!(
            dbg.contains("xoxb-***") && dbg.contains("xapp-***"),
            "prefix kept: {dbg}"
        );

        let cfg = Config {
            bot_token: Some("xoxb-SECRETBODY".into()),
            app_token: Some("xapp-SECRETBODY".into()),
            channel: Some("D0X".into()),
            fleet_dir: PathBuf::from("/tmp/f"),
            default_to: "concierge".into(),
            bridge_agent: "slack-bridge".into(),
        };
        let dbg = format!("{cfg:?}");
        assert!(
            !dbg.contains("SECRETBODY"),
            "config Debug must not leak tokens: {dbg}"
        );
        assert!(dbg.contains("D0X"), "non-secret fields still shown");
    }
}
