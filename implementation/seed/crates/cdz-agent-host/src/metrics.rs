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
//! **Surface, two halves:**
//! - [`HostMetrics`] — HOST-BOUNDARY events (session lifecycle + per-turn outcomes) that
//!   [`AgentHost`](crate::AgentHost) sees directly at `spawn`/`remove`/`deliver`.
//! - [`EffectMetrics`] — PER-EFFECT dispatch outcomes (ok / retryable-err / permanent-err). These happen
//!   DEEP in the kernel's deliver loop, not at the host boundary, so they're captured by a metering
//!   [`MeteredExecutor`](crate::factory::MeteredExecutor) decorator that wraps each real executor at
//!   registration and classifies every [`EffectOutcome`](cdz_kernel::event::EffectOutcome) via
//!   [`crate::retry::classify`] — no kernel change, no faked data. `EffectMetrics` is `Arc`-shared so all of
//!   a session's per-family decorators tally into one set the exporter reads.

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

/// Per-EFFECT dispatch counters — how the daemon's effects RESOLVED, split by the
/// [`retry`](crate::retry) classification of a failure. Fed by the
/// [`MeteredExecutor`](crate::factory::MeteredExecutor) decorator that wraps each real executor: on every
/// `perform`, it bumps exactly one of ok / retryable-err / permanent-err from the outcome. `Arc`-shared
/// (all of a session's per-family decorators tally into ONE set), so it's behind a shared reference and its
/// methods take `&self` (the atomics give interior mutability).
///
/// The retryable/permanent split is exactly what a supervision/retry driver acts on, so surfacing it as a
/// metric lets an operator SEE the transient-vs-fatal failure mix (a rising retryable rate = a flaky
/// upstream; a permanent spike = a misconfig/bad request) before the retry MECHANISM itself lands.
#[derive(Debug, Default)]
pub struct EffectMetrics {
    /// Effects that resolved `Ok`.
    effects_ok: AtomicU64,
    /// Effects that resolved `Err` with a `RETRYABLE:`-classified reason (a transient failure).
    effects_retryable_err: AtomicU64,
    /// Effects that resolved `Err` with a PERMANENT reason (incl. the fail-closed unprefixed default).
    effects_permanent_err: AtomicU64,
    /// Effects the kernel CANCELLED on deadline (`EffectOutcome::TimedOut`, §16c-S4) — a distinct failure
    /// mode from a classified `Err` (a hung effect that never returned a result), counted on its own.
    effects_timed_out: AtomicU64,
}

impl EffectMetrics {
    /// Record one successful effect dispatch.
    pub fn record_ok(&self) {
        self.effects_ok.fetch_add(1, Ordering::Relaxed);
    }

    /// Record one FAILED effect dispatch, split by its [`retry`](crate::retry) classification. The metering
    /// decorator classifies the `Err` reason with [`crate::retry::classify`] and passes the result here, so
    /// the retryable/permanent split matches exactly what a supervisor would branch on (fail-closed: an
    /// unprefixed reason classifies Permanent).
    pub fn record_err(&self, retryability: crate::retry::Retryability) {
        match retryability {
            crate::retry::Retryability::Retryable => {
                self.effects_retryable_err.fetch_add(1, Ordering::Relaxed);
            }
            crate::retry::Retryability::Permanent => {
                self.effects_permanent_err.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Record one effect the kernel cancelled on deadline (`EffectOutcome::TimedOut`).
    pub fn record_timed_out(&self) {
        self.effects_timed_out.fetch_add(1, Ordering::Relaxed);
    }

    /// A point-in-time copy of every effect counter.
    pub fn snapshot(&self) -> EffectMetricsSnapshot {
        EffectMetricsSnapshot {
            effects_ok: self.effects_ok.load(Ordering::Relaxed),
            effects_retryable_err: self.effects_retryable_err.load(Ordering::Relaxed),
            effects_permanent_err: self.effects_permanent_err.load(Ordering::Relaxed),
            effects_timed_out: self.effects_timed_out.load(Ordering::Relaxed),
        }
    }
}

/// A plain-value snapshot of [`EffectMetrics`] — what the status surface / exporter reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EffectMetricsSnapshot {
    /// Effects that resolved `Ok` (cumulative).
    pub effects_ok: u64,
    /// Effects that failed with a retryable (transient) reason (cumulative).
    pub effects_retryable_err: u64,
    /// Effects that failed with a permanent reason (cumulative; includes the unprefixed fail-closed default).
    pub effects_permanent_err: u64,
    /// Effects the kernel cancelled on deadline (cumulative).
    pub effects_timed_out: u64,
}

impl EffectMetricsSnapshot {
    /// Total effects dispatched = ok + retryable-err + permanent-err + timed-out.
    pub fn effects_total(&self) -> u64 {
        self.effects_ok
            .saturating_add(self.effects_retryable_err)
            .saturating_add(self.effects_permanent_err)
            .saturating_add(self.effects_timed_out)
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

    #[test]
    fn effect_metrics_split_ok_retryable_permanent() {
        use crate::retry::Retryability;
        let m = EffectMetrics::default();
        assert_eq!(m.snapshot(), EffectMetricsSnapshot::default());

        m.record_ok();
        m.record_ok();
        m.record_err(Retryability::Retryable);
        m.record_err(Retryability::Permanent);
        m.record_err(Retryability::Permanent);
        m.record_timed_out();

        let s = m.snapshot();
        assert_eq!(s.effects_ok, 2);
        assert_eq!(s.effects_retryable_err, 1);
        assert_eq!(s.effects_permanent_err, 2);
        assert_eq!(s.effects_timed_out, 1);
        assert_eq!(
            s.effects_total(),
            6,
            "total = ok + retryable + permanent + timed-out"
        );
    }
}
