//! The daemon's METRIC SURFACE — the counters a running host records into the s2n-quic-dc-metrics
//! REGISTRY, so observability can export them (operator seq-115/116: use the registry's metric types
//! directly, not bare atomics; we get histograms + the crate's reporting for free).
//!
//! [`HostMetrics`] and [`EffectMetrics`] are thin sets of registry [`Counter`]s (+ a latency [`Summary`]
//! each) registered from a shared [`Registry`]. The host loop increments them at the session-lifecycle /
//! per-turn / per-effect boundaries; the registry is what an export backend (statsd/otlp/…, a following
//! feature-gated slice) drains on its reporting interval via `Registry::report`. There is NO cumulative
//! poll surface — the earlier admin `metrics` read was removed (seq-117), so drain-on-report is exactly the
//! model (each counter accumulates then drains to the backend per interval).
//!
//! **Surface, two halves (both register into the daemon's one Registry):**
//! - [`HostMetrics`] — HOST-BOUNDARY events [`AgentHost`](crate::AgentHost) sees at `spawn`/`remove`/
//!   `deliver`: sessions installed/removed, turns ok/err, deliveries to an unknown session, + a turn-latency
//!   [`Summary`].
//! - [`EffectMetrics`] — PER-EFFECT dispatch outcomes captured by the
//!   [`MeteredExecutor`](crate::factory::MeteredExecutor) decorator: ok / retryable-err / permanent-err /
//!   timed-out (read off the outcome's typed [`Retryability`](cdz_kernel::event::Retryability)), + an
//!   effect-latency [`Summary`]. `Arc`-shared so
//!   all of a session's per-family decorators tally into one set.

use s2n_quic_dc_metrics::{Counter, Summary, Unit};

/// Re-export the registry type the host owns + hands to the exporter (so callers name it as
/// `crate::metrics::Registry` without depending on the metrics crate directly).
pub use s2n_quic_dc_metrics::Registry;

/// Saturating `Duration`-micros → `u64` for a latency [`Summary`] record. `Duration::as_micros` returns
/// `u128`; the `as u64` truncation is unreachable in practice (`u64::MAX` µs ≈ 584k years), but saturate
/// rather than silently wrap so an absurd/monotonic-clock-glitch duration clamps to the max instead of
/// wrapping to a small value (#2084 review; one helper for the host + effect latency sites).
pub(crate) fn micros_u64(d: std::time::Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

/// The metric name every host-boundary counter aggregates under (the aggregation string distinguishes the
/// individual counters via the `Variant|…` convention the s2n-quic-dc registry uses).
const HOST_METRIC: &str = "cdz_agent_host.session";
/// The metric name the per-effect counters aggregate under.
const EFFECT_METRIC: &str = "cdz_agent_host.effect";

/// The daemon's host-boundary metric counters — registry [`Counter`]s incremented on the host loop, drained
/// to the export backend by the registry's reporting interval. Built from the daemon's shared [`Registry`]
/// via [`new`](Self::new); a [`Counter`] is cheap to hold (an `Arc` into the registry). (Not `Debug` — the
/// registry's `Summary` isn't `Debug`.)
pub struct HostMetrics {
    sessions_installed: Counter,
    sessions_removed: Counter,
    turns_ok: Counter,
    turns_err: Counter,
    deliveries_to_unknown_session: Counter,
    /// Per-turn wall-clock latency (microseconds) — a distribution the registry renders as a histogram.
    turn_latency_us: Summary,
}

impl HostMetrics {
    /// Register the host-boundary metrics into `registry`. Each is one counter under the `session` metric,
    /// distinguished by a `Variant|<name>` aggregation (the s2n-quic-dc convention that lets a backend
    /// aggregate the set); the latency is a `Summary` (histogram).
    pub fn new(registry: &Registry) -> Self {
        HostMetrics {
            sessions_installed: registry.register_counter(
                HOST_METRIC.into(),
                Some("Variant|sessions-installed".into()),
            ),
            sessions_removed: registry
                .register_counter(HOST_METRIC.into(), Some("Variant|sessions-removed".into())),
            turns_ok: registry
                .register_counter(HOST_METRIC.into(), Some("Variant|turns-ok".into())),
            turns_err: registry
                .register_counter(HOST_METRIC.into(), Some("Variant|turns-err".into())),
            deliveries_to_unknown_session: registry.register_counter(
                HOST_METRIC.into(),
                Some("Variant|deliveries-to-unknown-session".into()),
            ),
            turn_latency_us: registry.register_summary(
                HOST_METRIC.into(),
                Some("Variant|turn-latency-us".into()),
                Unit::Microsecond,
            ),
        }
    }

    /// Record a session install (one successful `spawn`).
    pub(crate) fn record_session_installed(&self) {
        self.sessions_installed.increment(1);
    }

    /// Record a session removal (one `remove` that found a session, or a replaced spawn).
    pub(crate) fn record_session_removed(&self) {
        self.sessions_removed.increment(1);
    }

    /// Record one turn delivered to a known session, tagged by whether the turn succeeded — increments
    /// exactly one of `turns_ok` / `turns_err`.
    pub(crate) fn record_turn(&self, ok: bool) {
        if ok {
            self.turns_ok.increment(1);
        } else {
            self.turns_err.increment(1);
        }
    }

    /// Record a delivery addressed to an unknown session id (routed nowhere).
    pub(crate) fn record_delivery_to_unknown_session(&self) {
        self.deliveries_to_unknown_session.increment(1);
    }

    /// Record one turn's wall-clock duration (µs) into the latency histogram.
    pub(crate) fn record_turn_latency_us(&self, us: u64) {
        self.turn_latency_us.record_value(us);
    }
}

/// Per-EFFECT dispatch counters — how the daemon's effects RESOLVED, split by the typed [`Retryability`](cdz_kernel::event::Retryability)
/// classification of a failure, plus a dispatch-latency [`Summary`]. `Arc`-shared (all of a session's
/// per-family [`MeteredExecutor`](crate::factory::MeteredExecutor)s tally into ONE set), so its methods take
/// `&self`. (Not `Debug` — the registry's `Summary` isn't `Debug`.)
pub struct EffectMetrics {
    effects_ok: Counter,
    effects_retryable_err: Counter,
    effects_permanent_err: Counter,
    effects_timed_out: Counter,
    /// Per-effect dispatch latency (microseconds) — a histogram.
    effect_latency_us: Summary,
}

impl EffectMetrics {
    /// Register the per-effect metrics into `registry` (one counter per outcome under the `effect` metric,
    /// distinguished by `Variant|<outcome>`; the latency is a `Summary`).
    pub fn new(registry: &Registry) -> Self {
        EffectMetrics {
            effects_ok: registry.register_counter(EFFECT_METRIC.into(), Some("Variant|ok".into())),
            effects_retryable_err: registry
                .register_counter(EFFECT_METRIC.into(), Some("Variant|retryable-err".into())),
            effects_permanent_err: registry
                .register_counter(EFFECT_METRIC.into(), Some("Variant|permanent-err".into())),
            effects_timed_out: registry
                .register_counter(EFFECT_METRIC.into(), Some("Variant|timed-out".into())),
            effect_latency_us: registry.register_summary(
                EFFECT_METRIC.into(),
                Some("Variant|latency-us".into()),
                Unit::Microsecond,
            ),
        }
    }

    /// Record one successful effect dispatch.
    pub(crate) fn record_ok(&self) {
        self.effects_ok.increment(1);
    }

    /// Record one FAILED effect dispatch, split by its typed [`Retryability`](cdz_kernel::event::Retryability)
    /// (the metering decorator reads it straight off `EffectOutcome::Err { retryability, .. }`; the
    /// fail-closed default an un-annotated error carries is `Permanent`).
    pub(crate) fn record_err(&self, retryability: cdz_kernel::event::Retryability) {
        match retryability {
            cdz_kernel::event::Retryability::Retryable => self.effects_retryable_err.increment(1),
            cdz_kernel::event::Retryability::Permanent => self.effects_permanent_err.increment(1),
        }
    }

    /// Record one effect the kernel cancelled on deadline (`EffectOutcome::TimedOut`).
    pub(crate) fn record_timed_out(&self) {
        self.effects_timed_out.increment(1);
    }

    /// Record one effect dispatch's duration (µs) into the latency histogram.
    pub(crate) fn record_latency_us(&self, us: u64) {
        self.effect_latency_us.record_value(us);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Registry counters register + increment without panicking, and re-registering the same
    /// (metric, aggregation) is idempotent (the registry dedups by key), so building two metric sets from one
    /// registry is safe. We can't read a counter's value back out (drain-on-report, no getter), so the test
    /// asserts the record path is total (no panic) + the registry drives a report over them.
    #[test]
    fn host_and_effect_metrics_register_and_record_without_panic() {
        let registry = Registry::new();
        let host = HostMetrics::new(&registry);
        let effects = EffectMetrics::new(&registry);

        host.record_session_installed();
        host.record_session_removed();
        host.record_turn(true);
        host.record_turn(false);
        host.record_delivery_to_unknown_session();
        host.record_turn_latency_us(12);

        effects.record_ok();
        effects.record_err(cdz_kernel::event::Retryability::Retryable);
        effects.record_err(cdz_kernel::event::Retryability::Permanent);
        effects.record_timed_out();
        effects.record_latency_us(3);

        // Drive a querylog report over the registry — proves the registered metrics are reportable (drains
        // the accumulated values to a backend), the export model the exporter uses.
        let line = registry.try_take_current_metrics_line();
        assert!(
            line.is_some(),
            "the registry reports a metrics line over the registered counters"
        );
    }

    #[test]
    fn effect_metrics_is_arc_shareable_across_families() {
        // The daemon wraps each leaf executor sharing ONE Arc<EffectMetrics>; confirm &self methods work
        // through an Arc (the shape MeteredExecutor holds).
        let registry = Registry::new();
        let m = std::sync::Arc::new(EffectMetrics::new(&registry));
        let m2 = m.clone();
        m.record_ok();
        m2.record_timed_out();
        // No panic + both handles record into the same set.
    }

    #[test]
    fn micros_u64_converts_exactly_and_saturates() {
        use std::time::Duration;
        // A normal duration converts exactly (µs).
        assert_eq!(micros_u64(Duration::from_micros(0)), 0);
        assert_eq!(micros_u64(Duration::from_micros(1234)), 1234);
        assert_eq!(micros_u64(Duration::from_millis(5)), 5_000);
        // An absurd duration whose µs exceed u64::MAX SATURATES (does not wrap to a small value) — the
        // #2084-review concern the helper exists for. Duration::from_secs(u64::MAX) has ~1.8e25 µs >> u64::MAX.
        assert_eq!(micros_u64(Duration::from_secs(u64::MAX)), u64::MAX);
    }
}
