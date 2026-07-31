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
//! Invocation/cadence confirmed with v-fleet-tooling (owns the watchdog tool): fire `cargo xtask fleet
//! watchdog` (NO `--stale-mult` override — inherit the tool's `2 / 600s cap / 120s grace` defaults) every
//! ~4 min. See [`WatchdogSpec`] for why the override is `None`.

use std::time::Duration;

/// How often the daemon fires the watchdog: every ~4 minutes.
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(240);

/// Hard deadline for a SINGLE `cargo xtask fleet watchdog` fire. A bare await on the child has no
/// deadline, so one hung fire (a wedged cargo/git/tmux call, a shared-registry lock) would block the
/// daemon's fire loop indefinitely and silently halt ALL re-arms — the ~12h watchdog gap observed in
/// watchdog.log during the cred-freeze window. 180s is comfortably longer than a healthy fire (a sweep is
/// seconds of git/tmux/fs work — it does NOT compile) yet safely SHORTER than the 240s interval, so a
/// killed fire still leaves headroom before the next one is due. The runner kills the child on elapse and
/// continues, so a hung fire costs one cycle, never the daemon's liveness.
pub const FIRE_TIMEOUT: Duration = Duration::from_secs(180);

/// Grace for reaping a timed-out fire. On timeout the runner explicitly `kill().await`s the child so the
/// reap (`waitpid`) is timely — `kill_on_drop(true)` alone only SIGNALS the kill and defers the reap to
/// tokio's background orphan reaper (best-effort), so a repeatedly-hung watchdog could accumulate zombies
/// / leak PIDs (PR#949). The explicit kill+reap is itself bounded by this grace so a wedged reap can't
/// stall the fire loop. SIGKILL reaps near-instantly; a few seconds is ample, and FIRE_TIMEOUT + this
/// stays comfortably under the interval (see the headroom test).
pub const REAP_GRACE: Duration = Duration::from_secs(10);

/// A fully-specified watchdog invocation. Kept as data (program + args) so it's trivially unit-testable and
/// the transport just feeds it to `std::process::Command`. This is `cargo xtask fleet watchdog …` — we
/// shell out rather than call fleet.rs in-process so the watchdog tool stays the single source of truth
/// (its logic, guards, and thresholds live in one place that v-fleet-tooling owns).
///
/// `stale_mult` is `None` by DEFAULT: v-fleet-tooling (the tool owner) advised the continuous out-of-band
/// runner should use the tool's OWN defaults (`--stale-mult 2 --stale-cap 600 --grace-secs 120`) — NOT
/// `--stale-mult 1`, which over-eagerly re-arms a 10m agent after one missed interval; the `--stale-cap`
/// (600s) already bounds dead-time. So `None` passes no override and inherits those defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchdogSpec {
    /// `--stale-mult` override, or `None` to inherit the tool's default (the recommended setting).
    pub stale_mult: Option<u32>,
    /// How often the daemon fires the watchdog.
    pub interval: Duration,
}

impl Default for WatchdogSpec {
    fn default() -> Self {
        WatchdogSpec {
            stale_mult: None,
            interval: DEFAULT_INTERVAL,
        }
    }
}

impl WatchdogSpec {
    /// The program + argv to spawn: `cargo xtask fleet watchdog [--stale-mult <n>]`. With `stale_mult =
    /// None` (the default) it's just `cargo xtask fleet watchdog`, inheriting the tool's own thresholds.
    /// Returned as `(program, args)` so the caller builds a `Command` (and sets the cwd to the repo root).
    pub fn command(&self) -> (&'static str, Vec<String>) {
        let mut args = vec!["xtask".to_string(), "fleet".into(), "watchdog".into()];
        if let Some(mult) = self.stale_mult {
            args.push("--stale-mult".into());
            args.push(mult.to_string());
        }
        ("cargo", args)
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
    fn default_inherits_tool_thresholds() {
        // v-fleet-tooling's guidance: no --stale-mult override for the continuous runner.
        let s = WatchdogSpec::default();
        assert_eq!(s.stale_mult, None);
        assert_eq!(s.interval, Duration::from_secs(240));
    }

    #[test]
    fn default_command_passes_no_threshold_overrides() {
        let (prog, args) = WatchdogSpec::default().command();
        assert_eq!(prog, "cargo");
        assert_eq!(
            args,
            ["xtask", "fleet", "watchdog"],
            "no --stale-mult → inherit tool defaults"
        );
    }

    #[test]
    fn command_reflects_an_explicit_stale_mult_override() {
        let (_prog, args) = WatchdogSpec {
            stale_mult: Some(3),
            ..Default::default()
        }
        .command();
        assert_eq!(args, ["xtask", "fleet", "watchdog", "--stale-mult", "3"]);
    }

    #[test]
    fn fire_timeout_is_bounded_below_the_interval() {
        // The safety invariant: a single hung fire is killed BEFORE the next one is due, so the daemon's
        // fire cadence is never stalled by one wedged child (the ~12h watchdog-gap bug). Must be strictly
        // less than the interval, and positive.
        assert!(
            FIRE_TIMEOUT < DEFAULT_INTERVAL,
            "a fire must time out before the next is due ({:?} !< {:?})",
            FIRE_TIMEOUT,
            DEFAULT_INTERVAL
        );
        assert!(
            FIRE_TIMEOUT > Duration::from_secs(0),
            "timeout must be positive"
        );
    }

    #[test]
    fn fire_timeout_plus_reap_grace_stays_under_the_interval() {
        // The timeout arm does an explicit kill+reap bounded by REAP_GRACE (PR#949). Its worst case runs
        // AFTER the fire timeout, so the total time a wedged fire can hold the loop is FIRE_TIMEOUT +
        // REAP_GRACE — that must still be under the interval or a hung fire could push the next one late.
        assert!(
            REAP_GRACE > Duration::from_secs(0),
            "grace must be positive"
        );
        assert!(
            FIRE_TIMEOUT + REAP_GRACE < DEFAULT_INTERVAL,
            "fire timeout + reap grace must stay under the interval ({:?} + {:?} !< {:?})",
            FIRE_TIMEOUT,
            REAP_GRACE,
            DEFAULT_INTERVAL
        );
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
