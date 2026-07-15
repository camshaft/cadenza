//! Watchdog runner — the daemon's second job (operator decision, 2026-07-15).
//!
//! The bridge is a long-lived host process for the Slack Socket-Mode connection, so the operator wants it
//! to ALSO run the fleet watchdog out-of-band: `cargo xtask fleet watchdog` on a fixed cadence, re-arming
//! stalled `/loop`s and reaping stopped windows. This matters because the watchdog otherwise only runs in
//! the concierge's session-cron, which cannot heal a *concierge* stall (the concierge sat dead 4.3h once).
//! An independent host daemon can. This makes the bridge the fleet's reliability backbone.
//!
//! This module is the PURE part: the command SPEC (what to run) and the cadence DECISION (is it due yet),
//! both unit-tested with no async and no process spawning. The transport bin owns the actual `Command`
//! spawn + the real clock — a thin wrapper around [`WatchdogSpec::command`] fired when [`due`] says so.
//!
//! Exact invocation/cadence are being confirmed with v-fleet-tooling (owns the watchdog tool); the
//! defaults below encode the operator's stated baseline (`--stale-mult 1`, every ~4 min).

use std::time::Duration;

/// The operator's baseline cadence: run the watchdog every ~4 minutes.
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(240);
/// The operator's baseline `--stale-mult` for the out-of-band runner (more aggressive than a session-cron
/// default of 2, since this is the last line of defense against a fully-stalled fleet).
pub const DEFAULT_STALE_MULT: u32 = 1;

/// A fully-specified watchdog invocation. Kept as data (program + args) so it's trivially unit-testable and
/// the transport just feeds it to `std::process::Command`. This is `cargo xtask fleet watchdog …` — we
/// shell out rather than call fleet.rs in-process so the watchdog tool stays the single source of truth
/// (its logic, guards, and cadence live in one place that v-fleet-tooling owns).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchdogSpec {
    /// `--stale-mult`: presume a loop stalled once its heartbeat is older than this × its interval.
    pub stale_mult: u32,
    /// How often the daemon fires the watchdog.
    pub interval: Duration,
}

impl Default for WatchdogSpec {
    fn default() -> Self {
        WatchdogSpec {
            stale_mult: DEFAULT_STALE_MULT,
            interval: DEFAULT_INTERVAL,
        }
    }
}

impl WatchdogSpec {
    /// The program + argv to spawn: `cargo xtask fleet watchdog --stale-mult <n>`. Returned as
    /// `(program, args)` so the caller builds a `Command` (and can set the cwd to the repo root). Pure.
    pub fn command(&self) -> (&'static str, Vec<String>) {
        (
            "cargo",
            vec![
                "xtask".into(),
                "fleet".into(),
                "watchdog".into(),
                "--stale-mult".into(),
                self.stale_mult.to_string(),
            ],
        )
    }
}

/// Is the watchdog due to run? `elapsed_since_last` is how long since the last run (the transport tracks
/// this against the real clock — kept out of here so the decision is pure/testable). Fires when at least
/// `interval` has passed. A daemon that just started (no prior run) should pass `elapsed >= interval` (or
/// simply run once at startup) — expressed by the caller, not baked in here.
pub fn due(elapsed_since_last: Duration, interval: Duration) -> bool {
    elapsed_since_last >= interval
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_spec_matches_operator_baseline() {
        let s = WatchdogSpec::default();
        assert_eq!(s.stale_mult, 1);
        assert_eq!(s.interval, Duration::from_secs(240));
    }

    #[test]
    fn command_is_cargo_xtask_fleet_watchdog() {
        let (prog, args) = WatchdogSpec::default().command();
        assert_eq!(prog, "cargo");
        assert_eq!(args, ["xtask", "fleet", "watchdog", "--stale-mult", "1"]);
    }

    #[test]
    fn command_reflects_a_custom_stale_mult() {
        let (_prog, args) = WatchdogSpec {
            stale_mult: 3,
            ..Default::default()
        }
        .command();
        assert_eq!(args.last().unwrap(), "3");
    }

    #[test]
    fn due_fires_only_after_the_interval() {
        let iv = Duration::from_secs(240);
        assert!(!due(Duration::from_secs(0), iv), "just ran → not due");
        assert!(
            !due(Duration::from_secs(239), iv),
            "one sec short → not due"
        );
        assert!(due(Duration::from_secs(240), iv), "exactly due");
        assert!(due(Duration::from_secs(600), iv), "well past → due");
    }
}
