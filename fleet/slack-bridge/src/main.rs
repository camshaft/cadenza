//! Cadenza fleet ↔ Slack bridge daemon — the async transport (Socket Mode) that wires the pure core
//! (`config` + `inbox` + `format` + `sidecar` + `watchdog`) into a live process.
//!
//! Runs three concurrent jobs on one tokio runtime:
//!   1. INBOUND (Slack → fleet): a Socket Mode listener on Slack push events. A message the operator types
//!      in the bridge channel — or an in-thread reply to a mirrored ask — is turned into a fleet message
//!      and delivered into the target agent's inbox (which `cargo xtask fleet send` also auto-nudges).
//!   2. OUTBOUND (fleet → Slack): a poll loop that watches the concierge inbox, mirrors new `ask`/`backlog`
//!      to the channel as threads (recording `thread_ts → asker` in `slack-threads.json`), AND drains the
//!      bridge's own inbox to relay agent→operator messages.
//!   3. WATCHDOG: fires `cargo xtask fleet watchdog` on a cadence so this long-lived host heals stalled
//!      loops out-of-band (a concierge stall can't be healed by the concierge's own session-cron).
//!
//! FAIL-SOFT: with no Slack tokens ([`Config::tokens`] is `None`) the Slack jobs stay dormant, but the
//! WATCHDOG still runs — so the bridge is useful (as the reliability backbone) even before the operator
//! has created the Slack app. Any Slack/IO error is logged and retried, never a crash.
//!
//! This binary is intentionally thin: all the decisions live in the unit-tested lib. It is not itself
//! unit-tested (it's a live-WebSocket + subprocess shell); the gate is the lib's `cargo test`.

mod runner;

use cadenza_slack_bridge::{Config, ThreadMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn default_fleet_dir() -> PathBuf {
    // <repo>/fleet/slack-bridge/ → <repo>/.claude/fleet (the SHARED runtime dir with the inboxes).
    // Overridable via FLEET_DIR (needed when running from a worktree, whose fleet/ is tracked source).
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join(".claude")
        .join("fleet")
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cfg = Arc::new(Config::from_env(&default_fleet_dir()));
    tracing::info!(
        fleet_dir = %cfg.fleet_dir.display(),
        default_to = %cfg.default_to,
        bridge_agent = %cfg.bridge_agent,
        channel = ?cfg.channel,
        "cadenza fleet↔slack bridge starting"
    );

    // The watchdog runs regardless of Slack — it's the reliability backbone. Spawn it first.
    let watchdog_handle = {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..");
        tokio::spawn(runner::watchdog_loop(repo_root))
    };

    match cfg.tokens() {
        Some(tokens) => {
            tracing::info!("slack tokens present — starting Socket Mode + the outbound mirror");
            // The outbound mirror/relay poll loop.
            let outbound = tokio::spawn(runner::outbound_loop(cfg.clone(), tokens.clone()));
            // The inbound Socket Mode listener (blocks until the socket closes / a fatal error).
            if let Err(e) = runner::run_socket_mode(cfg.clone(), tokens).await {
                tracing::error!(error = %e, "socket mode listener exited");
            }
            outbound.abort();
        }
        None => {
            tracing::warn!(
                "no Slack tokens (set SLACK_BOT_TOKEN + SLACK_APP_TOKEN, or {}); \
                 running WATCHDOG-ONLY until they're provided",
                ThreadMap::path(&cfg.fleet_dir)
                    .parent()
                    .map(|p| p.join("slack.toml").display().to_string())
                    .unwrap_or_default()
            );
            // Dormant: keep the process alive so the watchdog keeps running; a restart picks up tokens.
            loop {
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
        }
    }

    let _ = watchdog_handle.await;
}
