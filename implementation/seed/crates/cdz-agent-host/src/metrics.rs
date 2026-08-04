//! The daemon's METRIC SURFACE — the counters a running host produces, so observability has something to
//! export. (Operator directive: a real daemon with metrics. Concierge-sequenced: "produce metrics → then
//! export" — this is the PRODUCE half; the s2n-quic-dc-metrics EXPORT half is a following, feature-gated
//! slice that bridges this surface to a [`MetricsTarget`](crate::MetricsTarget) backend.)
//!
//! [`HostMetrics`] is a hermetic, crate-local set of monotonic counters — plain [`AtomicU64`]s, NO metrics
//! dependency — so it lives in the DEFAULT build and the host loop increments it unconditionally (a counter
//! bump is a `Relaxed` add, free enough to always run). The exporter reads a [`HostMetricsSnapshot`] and maps
//! each counter onto the export backend's own metric type; nothing here knows about the backend.
//!
//! **Surface scope (v0): the HOST-BOUNDARY observable events** — session lifecycle + per-turn outcomes,
//! which [`AgentHost`](crate::AgentHost) sees directly at `spawn`/`remove`/`deliver`. Per-EFFECT dispatch
//! outcomes (ok / retryable / permanent, the [`crate::retry`] classification) happen DEEP in the kernel's
//! deliver loop (folded into the session log, not visible at the host boundary), so counting them needs an
//! executor-level reporting seam — a deliberate FOLLOW-UP slice, not faked here from data the host can't see.

use std::sync::atomic::{AtomicU64, Ordering};

/// The daemon's live metric counters — monotonic, host-boundary events. Held BY VALUE on the
/// [`AgentHost`](crate::AgentHost) (each counter is an [`AtomicU64`], so the struct is `Default` = all-zero
/// and needs no separate init). Incremented on the host loop; read via [`snapshot`](Self::snapshot) for a
/// status surface or the (feature-gated) metrics exporter. `Relaxed` ordering throughout — these are
/// independent counters with no happens-before relationship to protect (the single-threaded host loop is the
/// only writer; a snapshot reads a consistent-enough point-in-time, exact cross-counter atomicity is not a
/// metric requirement).
#[derive(Debug, Default)]
pub struct HostMetrics {
    /// Sessions installed into the registry (each successful [`AgentHost::spawn`](crate::AgentHost::spawn)).
    sessions_installed: AtomicU64,
    /// Sessions removed from the registry (each [`AgentHost::remove`](crate::AgentHost::remove) that found a
    /// session — a completed/closed agent dropped from the host).
    sessions_removed: AtomicU64,
    /// Turns delivered to a KNOWN session (each [`AgentHost::deliver`](crate::AgentHost::deliver) that routed
    /// to a live session, regardless of the turn's outcome). Equals `turns_ok + turns_err`.
    turns_delivered: AtomicU64,
    /// Turns that completed successfully (`deliver` returned `Ok`).
    turns_ok: AtomicU64,
    /// Turns that erred (`deliver` returned a `KernelError` — a fold/loop failure on that turn).
    turns_err: AtomicU64,
    /// Deliveries addressed to an UNKNOWN session id (routed to no session) — a misrouted/late event, worth
    /// surfacing distinctly from a delivered turn.
    deliveries_to_unknown_session: AtomicU64,
}

impl HostMetrics {
    /// Record a session install (one successful `spawn`).
    pub fn record_session_installed(&self) {
        self.sessions_installed.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a session removal (one `remove` that found a session).
    pub fn record_session_removed(&self) {
        self.sessions_removed.fetch_add(1, Ordering::Relaxed);
    }

    /// Record one turn delivered to a known session, tagged by whether the turn succeeded. Bumps
    /// `turns_delivered` plus exactly one of `turns_ok` / `turns_err`, so the ok+err split always sums to the
    /// delivered total.
    pub fn record_turn(&self, ok: bool) {
        self.turns_delivered.fetch_add(1, Ordering::Relaxed);
        if ok {
            self.turns_ok.fetch_add(1, Ordering::Relaxed);
        } else {
            self.turns_err.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record a delivery addressed to an unknown session id (routed nowhere).
    pub fn record_delivery_to_unknown_session(&self) {
        self.deliveries_to_unknown_session
            .fetch_add(1, Ordering::Relaxed);
    }

    /// A point-in-time copy of every counter — plain `u64`s the status surface / metrics exporter reads
    /// without touching the atomics directly. (Not cross-counter atomic; see the type-level note.)
    pub fn snapshot(&self) -> HostMetricsSnapshot {
        HostMetricsSnapshot {
            sessions_installed: self.sessions_installed.load(Ordering::Relaxed),
            sessions_removed: self.sessions_removed.load(Ordering::Relaxed),
            turns_delivered: self.turns_delivered.load(Ordering::Relaxed),
            turns_ok: self.turns_ok.load(Ordering::Relaxed),
            turns_err: self.turns_err.load(Ordering::Relaxed),
            deliveries_to_unknown_session: self
                .deliveries_to_unknown_session
                .load(Ordering::Relaxed),
        }
    }
}

/// A plain-value snapshot of [`HostMetrics`] — what a status query returns and the (feature-gated) exporter
/// maps onto its backend's metric types. Cloneable/comparable so a test asserts exact counts and the exporter
/// can diff successive snapshots (deltas) if it emits rates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HostMetricsSnapshot {
    /// Sessions installed (cumulative).
    pub sessions_installed: u64,
    /// Sessions removed (cumulative).
    pub sessions_removed: u64,
    /// Turns delivered to a known session (cumulative; equals `turns_ok + turns_err`).
    pub turns_delivered: u64,
    /// Turns that completed `Ok`.
    pub turns_ok: u64,
    /// Turns that returned a `KernelError`.
    pub turns_err: u64,
    /// Deliveries addressed to an unknown session id.
    pub deliveries_to_unknown_session: u64,
}

impl HostMetricsSnapshot {
    /// Sessions currently live per this snapshot = installed − removed. Saturating (never underflows) —
    /// a defensive total answer even if a snapshot straddles a remove-without-prior-install (which the host
    /// loop's ordering makes impossible in practice, but a saturating diff needs no invariant to be safe).
    pub fn sessions_live(&self) -> u64 {
        self.sessions_installed
            .saturating_sub(self.sessions_removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_metrics_set_is_all_zero() {
        let m = HostMetrics::default();
        assert_eq!(m.snapshot(), HostMetricsSnapshot::default());
        assert_eq!(m.snapshot().sessions_live(), 0);
    }

    #[test]
    fn counters_accumulate_and_snapshot_reflects_them() {
        let m = HostMetrics::default();
        m.record_session_installed();
        m.record_session_installed();
        m.record_session_removed();
        m.record_turn(true);
        m.record_turn(true);
        m.record_turn(false);
        m.record_delivery_to_unknown_session();

        let s = m.snapshot();
        assert_eq!(s.sessions_installed, 2);
        assert_eq!(s.sessions_removed, 1);
        assert_eq!(s.turns_delivered, 3, "delivered = ok + err");
        assert_eq!(s.turns_ok, 2);
        assert_eq!(s.turns_err, 1);
        assert_eq!(s.deliveries_to_unknown_session, 1);
        // The ok/err split always sums to the delivered total.
        assert_eq!(s.turns_ok + s.turns_err, s.turns_delivered);
        // Live = installed − removed.
        assert_eq!(s.sessions_live(), 1);
    }

    #[test]
    fn sessions_live_saturates_never_underflows() {
        // A snapshot with more removals than installs (impossible in the real host ordering, but the diff
        // must be total) pins at 0, not a wrapped huge number.
        let s = HostMetricsSnapshot {
            sessions_installed: 1,
            sessions_removed: 3,
            ..HostMetricsSnapshot::default()
        };
        assert_eq!(s.sessions_live(), 0);
    }
}
