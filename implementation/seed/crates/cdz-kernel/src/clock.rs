//! The wall clock (minimal-kernel re-charter, capability + scheduling substrate — operator ruling).
//!
//! Operator ruling (Slack): "the log needs to have a wall clock. That's fine. We also need it for one shot and
//! periodic events as well." The wall clock is the SHARED substrate for two features:
//!   • AUTHZ EXPIRY — an operator grant can carry an absolute expiry time; the effective-policy fold drops a
//!     grant once `now >= expiry` (time-boxed capabilities).
//!   • SCHEDULED EVENTS — one-shot ("in X time, perform this") + periodic ("every X min until cancelled"):
//!     cron/timer semantics on the agent-log, keyed on the same clock.
//!
//! Design (concierge resolution): the DAEMON reads the wall clock at tick time (an ambient side effect lives
//! only here); everything downstream takes TIME AS DATA — a `now_ms` argument threaded into the pure folds
//! (`effective_policy(events, now_ms)`, the scheduler's due-check) — so those stay pure + deterministically
//! testable (pass a fixed `now_ms`), and only this module touches the real clock. Milliseconds since the Unix
//! epoch (`u64`) is the unit: absolute, monotonic-enough for expiry/scheduling, and log-serializable.

/// Milliseconds since the Unix epoch, read from the system wall clock. The daemon calls this ONCE per tick and
/// threads the value into the pure folds as `now_ms` — the single ambient-time read in the kernel. A clock set
/// before the epoch (should never happen) clamps to 0 rather than panicking.
pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Whether an absolute `expiry_ms` (ms since the Unix epoch) has passed as of `now_ms`. A grant/schedule with
/// `expiry_ms <= now_ms` is expired/due. `None` = NO expiry (never expires) → always `false`. Pure: `now_ms`
/// is passed in (the caller read [`now_millis`] at the tick), so this is deterministically testable.
pub fn is_expired(expiry_ms: Option<u64>, now_ms: u64) -> bool {
    match expiry_ms {
        Some(exp) => exp <= now_ms,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_millis_is_a_plausible_recent_epoch_time() {
        // A sanity floor: the wall clock is past 2020-01-01 (1_577_836_800_000 ms) — proves it reads a real
        // epoch time, not 0. (Not an exact-value test — the clock advances; just that it's a real recent time.)
        let now = now_millis();
        assert!(
            now > 1_577_836_800_000,
            "now_millis reads a real post-2020 wall-clock time; got {now}"
        );
    }

    #[test]
    fn is_expired_honors_absolute_expiry_and_no_expiry() {
        // expiry <= now → expired; expiry > now → live; None → never expires.
        assert!(
            is_expired(Some(100), 100),
            "expiry == now → expired (boundary is inclusive)"
        );
        assert!(is_expired(Some(100), 200), "expiry < now → expired");
        assert!(!is_expired(Some(200), 100), "expiry > now → still live");
        assert!(
            !is_expired(None, u64::MAX),
            "no expiry → never expires, even at max now"
        );
    }
}
