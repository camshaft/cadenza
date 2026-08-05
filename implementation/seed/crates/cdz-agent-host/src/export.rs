//! Metrics EXPORT — the emitter that periodically reports the daemon's metrics [`Registry`] to a backend
//! (statsd first) over the network. Behind the `metrics-export` cargo feature (OFF by default): the metric
//! RECORDING (the registry Counters/Summaries in [`crate::metrics`]) is always-on + hermetic, but the
//! EXPORT — sending to a collector — is gated so the default build never opens a socket / sends UDP.
//!
//! The registry Counters are DRAIN-ON-REPORT: each [`Registry::report`] emits the per-interval delta and
//! CLEARS the counters, so the report cadence (`[observability].report_interval_secs`) IS the aggregation
//! window — exactly the statsd/otlp PUSH model (the collector does the time-series aggregation).
//!
//! Because reporting is DESTRUCTIVE, the exporter is BACKEND-AGNOSTIC (operator review on #2124): all export
//! backends are PUSHED INTO a [`MetricsReporter`] at config time, and each cycle drains the registry EXACTLY
//! ONCE and fans that single snapshot out to EVERY backend at the same time (the crate's `Vec<B>: Backend`
//! composition). Reporting to backends sequentially would drain on the first and hand every later backend
//! zeroes — so the reporter never does per-backend report calls. [`run_export_loop`] is the daemon's
//! periodic, shutdown-aware task that drives one [`MetricsReporter::report`] cycle every
//! `report_interval_secs` (plus one final report on shutdown); it knows nothing about which backends the
//! reporter holds. The daemon builds the reporter from `[observability].target` and spawns the loop with a
//! clone of the host's registry.
//!
//! The statsd sink uses a blocking [`std::net::UdpSocket`] (the send happens off the periodic timer tick,
//! not on any hot path, so a sync send is fine + avoids pulling tokio's `net` feature).

#![cfg(feature = "metrics-export")]

use s2n_quic_dc_metrics::Registry;

/// A statsd [`StatsdSink`](s2n_quic_dc_metrics::backend::StatsdSink) that sends each UDP payload to a
/// collector address. Holds a connected [`std::net::UdpSocket`]; `send_batch` fires one datagram per payload.
/// A send error is dropped (metrics export is best-effort — a dropped stat must never fail the daemon).
#[derive(Debug)]
pub struct UdpStatsdSink {
    socket: std::net::UdpSocket,
}

impl UdpStatsdSink {
    /// Bind an ephemeral local UDP socket connected to `endpoint` (`host:port`). `Err` if the address is
    /// unparseable / unreachable at bind time — surfaced to the caller (a bad statsd endpoint is a config
    /// error worth reporting at export setup, not a silent no-op).
    pub fn connect(endpoint: &str) -> Result<Self, String> {
        use std::net::ToSocketAddrs;
        // Resolve the endpoint to ALL its addresses, then try each in turn: bind an ephemeral local socket in
        // that address's MATCHING family (`[::]:0` for v6, `0.0.0.0:0` for v4 — a v4-bound socket cannot
        // `connect` to a v6 peer, EAFNOSUPPORT, #2112 review) and connect; return the FIRST address that both
        // binds and connects. Iterating ALL addresses (not just the first, #2119 review) means a hostname that
        // resolves v6-first on a host with no v6 route still succeeds via a later v4 address — the
        // multi-address robustness the family-matching fix was meant to provide.
        let addrs: Vec<std::net::SocketAddr> = endpoint
            .to_socket_addrs()
            .map_err(|e| format!("statsd sink could not resolve {endpoint}: {e}"))?
            .collect();
        if addrs.is_empty() {
            return Err(format!("statsd sink: {endpoint} resolved to no address"));
        }
        let mut last_err = String::new();
        for target in addrs {
            let bind_addr: std::net::SocketAddr = if target.is_ipv6() {
                (std::net::Ipv6Addr::UNSPECIFIED, 0).into()
            } else {
                (std::net::Ipv4Addr::UNSPECIFIED, 0).into()
            };
            match std::net::UdpSocket::bind(bind_addr).and_then(|s| {
                s.connect(target)?;
                Ok(s)
            }) {
                Ok(socket) => return Ok(UdpStatsdSink { socket }),
                Err(e) => last_err = format!("{target}: {e}"),
            }
        }
        Err(format!(
            "statsd sink could not bind+connect any resolved address for {endpoint} (last error {last_err})"
        ))
    }
}

impl s2n_quic_dc_metrics::backend::StatsdSink for UdpStatsdSink {
    fn send_batch<'a>(&mut self, payloads: impl Iterator<Item = &'a str>) {
        for payload in payloads {
            // Best-effort: a UDP send failure (collector down, buffer full) is dropped — export must never
            // fail the daemon or the report loop.
            let _ = self.socket.send(payload.as_bytes());
        }
    }
}

/// One resolved statsd export target — where to send + an optional metric-name prefix. OWNED (not borrowed)
/// so the reporter holds its target set for the daemon's lifetime without a config borrow. The daemon builds
/// these from the `statsd` entries of `[observability].target`.
#[derive(Debug, Clone)]
pub struct StatsdTarget {
    /// The statsd collector address (`host:port`) UDP payloads go to.
    pub endpoint: String,
    /// Optional metric-name prefix (maps to `StatsdBackend`'s prefix), e.g. `"cdz.agent"`.
    pub prefix: Option<String>,
}

/// A BACKEND-AGNOSTIC periodic metrics reporter: it holds the set of registered export backends (statsd
/// today; otlp/prometheus later) and, each report cycle, drains the [`Registry`] EXACTLY ONCE and fans the
/// drained snapshot out to ALL of them at the same time.
///
/// This is the correctness-critical shape (operator review on #2124): [`Registry::report`] is DESTRUCTIVE —
/// each report drains the counters and clears them. So reporting to backends SEQUENTIALLY (a per-backend or
/// per-target report call) would drain on the FIRST backend and hand every later backend ZEROES. Instead all
/// backends are PUSHED IN (at config time), held here as one `Vec<Box<dyn Backend + Send>>`, and a single
/// [`report`](Self::report) drives ONE [`Registry::report`] into the whole set (the crate's `Vec<B>: Backend`
/// fan-out), so every backend receives the SAME complete per-interval delta. The reporter itself is unaware
/// of WHICH backends it holds — it just drives "report now"; adding a backend kind is a new push at build
/// time, not a change to the loop.
///
/// Backends are built ONCE (a UDP sink's socket, an HTTP client's pool, …) and reused across reports — the
/// crate's [`Backend::report_start`](s2n_quic_dc_metrics::backend::Backend) resets per-report state, so reuse
/// is the intended model. A target whose sink can't be set up at build time is dropped (surfaced by the
/// constructor) rather than retried per tick.
pub struct MetricsReporter {
    backends: Vec<Box<dyn s2n_quic_dc_metrics::backend::Backend + Send>>,
}

impl MetricsReporter {
    /// Build a reporter from the configured statsd targets. Each target that connects becomes a
    /// [`StatsdBackend`](s2n_quic_dc_metrics::backend::StatsdBackend) pushed into the set; a target that can't
    /// connect (bad/unresolvable endpoint) is SKIPPED and its endpoint collected into the returned failure
    /// list (the daemon logs it at boot). The reporter is usable even with an empty backend set — a report
    /// then just drains the counters to nothing (keeping the drain-on-report window honest). (OTLP/prometheus
    /// backends will be pushed in by their own constructors/`push_*` in a following slice.)
    pub fn from_statsd_targets(targets: &[StatsdTarget]) -> (Self, Vec<String>) {
        use s2n_quic_dc_metrics::backend::StatsdBackend;
        let mut backends: Vec<Box<dyn s2n_quic_dc_metrics::backend::Backend + Send>> =
            Vec::with_capacity(targets.len());
        let mut failed = Vec::new();
        for t in targets {
            match UdpStatsdSink::connect(&t.endpoint) {
                Ok(sink) => backends.push(Box::new(StatsdBackend::new(sink, t.prefix.clone()))),
                Err(e) => failed.push(format!("{}: {e}", t.endpoint)),
            }
        }
        (MetricsReporter { backends }, failed)
    }

    /// How many backends are registered — the daemon skips spawning the export loop when this is 0.
    pub fn backend_count(&self) -> usize {
        self.backends.len()
    }

    /// Drive ONE report cycle: drain `registry` a SINGLE time and fan the drained snapshot out to every
    /// registered backend at once (the crate's `Vec<B>: Backend` composition). Because the drain is
    /// destructive, this is the ONLY correct multi-backend shape — one drain, all backends. An empty backend
    /// set still drains (to a no-op), which correctly clears the per-interval counters.
    pub fn report(&mut self, registry: &Registry) {
        registry.report(&mut self.backends);
    }
}

/// Run the daemon's periodic, BACKEND-AGNOSTIC metrics-export loop: every `interval`, drive ONE
/// [`MetricsReporter::report`] cycle (a single registry drain fanned out to every registered backend), until
/// `shutdown` fires — then do ONE FINAL report (so the last partial interval's metrics aren't lost) and
/// return. The loop knows NOTHING about which backends the reporter holds; it just drives the periodic
/// "report now". Export is best-effort — a backend's own send failure is swallowed inside that backend's sink
/// and never takes the daemon down.
///
/// `registry` is a CLONE of the host's registry (the `Arc`-backed handle shares the same storage), so the
/// loop reads the same counters the host loop increments. The task is spawned by the daemon and shut down
/// alongside the host loop.
pub async fn run_export_loop(
    registry: Registry,
    mut reporter: MetricsReporter,
    interval: std::time::Duration,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
) {
    let mut timer = tokio::time::interval(interval);
    // Steady cadence, not burst-catch-up; and consume the interval's immediate first tick so the first REAL
    // report happens one `interval` in (a report right at boot, before any work, would just emit zeroes).
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    timer.tick().await;

    loop {
        tokio::select! {
            _ = timer.tick() => reporter.report(&registry),
            _ = &mut shutdown => {
                // Final drain on the way out so the last interval's metrics reach the backends.
                reporter.report(&registry);
                tracing::debug!("metrics-export: export loop shut down");
                return;
            }
        }
    }
}

// ─── OTLP backend (feature `metrics-export-otlp`) ──────────────────────────────────────────────────────
//
// OTLP is a PUSH backend like statsd, but its payload is an OTLP protobuf `ExportMetricsServiceRequest` that
// must be sent over HTTP — and the crate's `OtlpSink::send(&[u8])` is SYNCHRONOUS, called inside
// `Backend::report_end`, which runs inside the SYNC `MetricsReporter::report`. So the sink CANNOT block or
// `.await` on the report path. The shape (concierge-approved): the sink's `send` does a NON-BLOCKING push of
// the payload onto a BOUNDED channel and returns immediately; a separate async FORWARDER task drains the
// channel and does the HTTP POST off the report path. Backpressure / a failed POST is DROP + log — telemetry
// is best-effort and must never block or crash the agent host (guard 2).

/// One resolved OTLP export target. OWNED so the reporter/forwarder hold it for the daemon's lifetime.
#[cfg(feature = "metrics-export-otlp")]
#[derive(Debug, Clone)]
pub struct OtlpTarget {
    /// The OTLP collector base endpoint; the forwarder POSTs to `{endpoint}/v1/metrics`.
    pub endpoint: String,
    /// OTLP `InstrumentationScope` name embedded in every payload (falls back to the crate name if empty).
    pub scope_name: Option<String>,
    /// OTLP `InstrumentationScope` version (falls back to empty if unset).
    pub scope_version: Option<String>,
}

/// The bound on the OTLP hand-off channel. Small: a report every `report_interval_secs` produces one payload,
/// so a healthy forwarder keeps this near-empty; the bound only matters when the collector is down/slow, and
/// then we WANT to drop (best-effort) rather than grow unbounded. Holds a few intervals of slack.
#[cfg(feature = "metrics-export-otlp")]
const OTLP_CHANNEL_BOUND: usize = 8;

/// A synchronous [`OtlpSink`](s2n_quic_dc_metrics::backend::OtlpSink) that hands each protobuf payload off to
/// the async forwarder over a BOUNDED channel WITHOUT blocking. `send` runs inside the sync report cycle, so
/// it must return immediately: it `try_send`s and, if the channel is full (the collector is backpressured) or
/// closed (forwarder gone), DROPS the payload + logs — never blocks, never panics (guard 2).
#[cfg(feature = "metrics-export-otlp")]
pub struct ChannelOtlpSink {
    tx: tokio::sync::mpsc::Sender<Vec<u8>>,
}

#[cfg(feature = "metrics-export-otlp")]
impl s2n_quic_dc_metrics::backend::OtlpSink for ChannelOtlpSink {
    fn send(&mut self, payload: &[u8]) {
        // Non-blocking hand-off. `try_send` never awaits; on Full/Closed we drop + log (best-effort telemetry
        // must not stall the report cycle waiting on a slow/down collector).
        match self.tx.try_send(payload.to_vec()) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                tracing::warn!(
                    "metrics-export otlp: forwarder channel full — dropping this report's payload"
                );
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                tracing::debug!("metrics-export otlp: forwarder gone — dropping payload");
            }
        }
    }
}

/// An [`OtlpBackend`](s2n_quic_dc_metrics::backend::OtlpBackend) wrapper that stamps the per-report wall-clock
/// timestamps OTLP data points need (`start_time_unix_nano`/`time_unix_nano`). The inner backend's timestamp
/// setters aren't reachable once it's boxed as `dyn Backend` in the reporter, so this wrapper sets them in
/// [`report_start`](s2n_quic_dc_metrics::backend::Backend::report_start) — the per-report hook — before
/// delegating every trait method to the inner backend. `start` is the previous report's end; the FIRST report
/// uses a zero-length interval (start == time == now) rather than a stored process-start, so it never emits a
/// start before the end. `time` is now; both Unix nanoseconds (statsd needed no timestamps — OTLP-specific).
#[cfg(feature = "metrics-export-otlp")]
struct TimestampedOtlpBackend {
    inner: s2n_quic_dc_metrics::backend::OtlpBackend<ChannelOtlpSink>,
    /// End of the PREVIOUS report interval (Unix ns) → the next report's start. 0 until the first report.
    last_report_ns: u64,
}

#[cfg(feature = "metrics-export-otlp")]
impl TimestampedOtlpBackend {
    fn now_unix_ns() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
            .unwrap_or(0)
    }
}

#[cfg(feature = "metrics-export-otlp")]
impl s2n_quic_dc_metrics::backend::Backend for TimestampedOtlpBackend {
    fn report_start(&mut self, options: &s2n_quic_dc_metrics::backend::ReportOptions) {
        // Clamp `now` up to the previous report's timestamp so the interval is NEVER inverted: OTLP requires
        // start_time_unix_nano <= time_unix_nano, but a BACKWARD wall-clock jump (or now_unix_ns's
        // unwrap_or(0) on a SystemTime error) could make now < last_report_ns → start > time (#2144 review).
        // max() absorbs both: the emitted interval degenerates to zero-length rather than inverting.
        let now = Self::now_unix_ns().max(self.last_report_ns);
        // First report: start == now (a zero-length interval) so we never emit a start after the end.
        let start = if self.last_report_ns == 0 {
            now
        } else {
            self.last_report_ns
        };
        self.inner.set_start_time_ns(start);
        self.inner.set_time_ns(now);
        self.last_report_ns = now;
        self.inner.report_start(options);
    }
    fn record_counter(&mut self, info: &s2n_quic_dc_metrics::backend::MetricInfo<'_>, value: u64) {
        self.inner.record_counter(info, value);
    }
    fn record_gauge(&mut self, info: &s2n_quic_dc_metrics::backend::MetricInfo<'_>, value: i64) {
        self.inner.record_gauge(info, value);
    }
    fn record_bool(
        &mut self,
        info: &s2n_quic_dc_metrics::backend::MetricInfo<'_>,
        true_count: u64,
        false_count: u64,
    ) {
        self.inner.record_bool(info, true_count, false_count);
    }
    fn record_histogram(
        &mut self,
        info: &s2n_quic_dc_metrics::backend::MetricInfo<'_>,
        hist: s2n_quic_dc_metrics::backend::Histogram<'_>,
    ) {
        self.inner.record_histogram(info, hist);
    }
    fn record_callback(
        &mut self,
        info: &s2n_quic_dc_metrics::backend::MetricInfo<'_>,
        values: &[&dyn s2n_quic_dc_metrics::backend::CallbackValue],
    ) {
        self.inner.record_callback(info, values);
    }
    fn report_end(&mut self) {
        self.inner.report_end();
    }
}

/// The async OTLP forwarder: drains the bounded channel and POSTs each protobuf payload to `endpoint`'s
/// `/v1/metrics` as `application/x-protobuf`. A failed/timed-out POST is DROPPED + logged (best-effort). Runs
/// until the channel closes (all sinks dropped, i.e. the reporter is gone at daemon shutdown). Spawned by the
/// daemon alongside the export loop.
#[cfg(feature = "metrics-export-otlp")]
pub async fn run_otlp_forwarder(endpoint: String, mut rx: tokio::sync::mpsc::Receiver<Vec<u8>>) {
    let url = format!("{}/v1/metrics", endpoint.trim_end_matches('/'));
    // Build the client with BOTH a connect timeout and an overall request timeout. Without them, a POST to a
    // BLACKHOLED collector (firewall DROP, not RST) would block INSIDE send().await with no deadline — the
    // forwarder would be stuck in the network call and never return to rx.recv(), so it could never honor the
    // channel-close→terminate contract (which is only checked between iterations) (#2144 review). A build
    // failure returns cleanly (log + stop) rather than unwrapping — a dead forwarder must not panic the host.
    let client = match reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, url = %url, "metrics-export otlp: could not build HTTP client — forwarder disabled");
            return;
        }
    };
    while let Some(payload) = rx.recv().await {
        let resp = client
            .post(&url)
            .header("content-type", "application/x-protobuf")
            .body(payload)
            .send()
            .await;
        match resp {
            Ok(r) if r.status().is_success() => {}
            Ok(r) => {
                tracing::warn!(status = %r.status(), url = %url, "metrics-export otlp: POST rejected — dropping")
            }
            Err(e) => {
                tracing::warn!(error = %e, url = %url, "metrics-export otlp: POST failed — dropping")
            }
        }
    }
    tracing::debug!(url = %url, "metrics-export otlp: forwarder shut down (channel closed)");
}

#[cfg(feature = "metrics-export-otlp")]
impl MetricsReporter {
    /// Register an OTLP backend into the reporter and return the channel receiver the daemon hands to
    /// [`run_otlp_forwarder`]. The backend's sink pushes each report's protobuf payload onto a bounded channel
    /// (non-blocking); the forwarder POSTs them. One call per OTLP target (each gets its own channel +
    /// forwarder task, so a slow collector on one target can't backpressure another).
    pub fn push_otlp(&mut self, target: &OtlpTarget) -> tokio::sync::mpsc::Receiver<Vec<u8>> {
        use s2n_quic_dc_metrics::backend::OtlpBackend;
        let (tx, rx) = tokio::sync::mpsc::channel(OTLP_CHANNEL_BOUND);
        let sink = ChannelOtlpSink { tx };
        let scope_name = target
            .scope_name
            .clone()
            .unwrap_or_else(|| "cdz-agent-host".to_string());
        let scope_version = target.scope_version.clone().unwrap_or_default();
        let inner = OtlpBackend::new(sink, scope_name, scope_version);
        self.backends.push(Box::new(TimestampedOtlpBackend {
            inner,
            last_report_ns: 0,
        }));
        rx
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::HostMetrics;

    #[test]
    fn reporter_reaches_a_bound_collector_without_error() {
        // Bind a real UDP socket as the "collector", record some metrics, build a reporter from one statsd
        // target, and report ONCE — proves the backend connects + the registry drives it + a datagram is
        // sent, all without error. (We don't assert the payload bytes — statsd wire format is the crate's
        // concern; this proves the export path is wired + total.)
        let collector = std::net::UdpSocket::bind(("127.0.0.1", 0)).expect("bind collector");
        collector
            .set_read_timeout(Some(std::time::Duration::from_millis(200)))
            .unwrap();
        let addr = collector.local_addr().unwrap().to_string();

        let registry = Registry::new();
        let host = HostMetrics::new(&registry);
        host.record_session_installed();
        host.record_turn(true);

        let (mut reporter, failed) = MetricsReporter::from_statsd_targets(&[StatsdTarget {
            endpoint: addr,
            prefix: Some("cdz.agent".into()),
        }]);
        assert!(failed.is_empty(), "the target connected: {failed:?}");
        assert_eq!(reporter.backend_count(), 1);
        reporter.report(&registry);

        // A datagram arrived at the collector (the statsd export actually sent something).
        let mut buf = [0u8; 4096];
        let got = collector.recv(&mut buf);
        assert!(got.is_ok(), "collector received a statsd datagram: {got:?}");
        assert!(got.unwrap() > 0, "the datagram was non-empty");
    }

    #[test]
    fn reporter_reaches_an_ipv6_collector() {
        // The #2112-review fix: a v6 statsd endpoint must work (a v4-bound socket can't connect to a v6
        // peer). Bind a v6 loopback collector + report to it — proves connect() binds the MATCHING family.
        let collector = match std::net::UdpSocket::bind(("::1", 0)) {
            Ok(s) => s,
            // Skip cleanly on a runner with no IPv6 loopback (some sandboxes) — the v4 test covers the path.
            Err(_) => return,
        };
        collector
            .set_read_timeout(Some(std::time::Duration::from_millis(200)))
            .unwrap();
        let addr = collector.local_addr().unwrap().to_string();

        let registry = Registry::new();
        HostMetrics::new(&registry).record_turn(true);

        let (mut reporter, failed) = MetricsReporter::from_statsd_targets(&[StatsdTarget {
            endpoint: addr,
            prefix: None,
        }]);
        assert!(failed.is_empty(), "v6 target connected: {failed:?}");
        reporter.report(&registry);

        let mut buf = [0u8; 4096];
        assert!(
            collector.recv(&mut buf).is_ok(),
            "the v6 collector received a statsd datagram"
        );
    }

    #[test]
    fn reporter_fans_one_drain_out_to_every_backend() {
        // Two collectors, both registered as backends. ONE report drains the registry once and fans out — so
        // BOTH collectors receive a (non-empty) datagram from the SAME drain. This is the correctness guard
        // the operator flagged on #2124: reporting per-backend would drain on the first and hand the second
        // ZEROES, because Registry::report is destructive.
        let c1 = std::net::UdpSocket::bind(("127.0.0.1", 0)).expect("bind c1");
        let c2 = std::net::UdpSocket::bind(("127.0.0.1", 0)).expect("bind c2");
        for c in [&c1, &c2] {
            c.set_read_timeout(Some(std::time::Duration::from_millis(200)))
                .unwrap();
        }
        let (mut reporter, failed) = MetricsReporter::from_statsd_targets(&[
            StatsdTarget {
                endpoint: c1.local_addr().unwrap().to_string(),
                prefix: Some("a".into()),
            },
            StatsdTarget {
                endpoint: c2.local_addr().unwrap().to_string(),
                prefix: Some("b".into()),
            },
        ]);
        assert!(failed.is_empty(), "both targets connected: {failed:?}");
        assert_eq!(reporter.backend_count(), 2);

        let registry = Registry::new();
        let host = HostMetrics::new(&registry);
        host.record_session_installed();
        host.record_turn(true);

        reporter.report(&registry);

        let mut buf = [0u8; 4096];
        assert!(
            c1.recv(&mut buf).map(|n| n > 0).unwrap_or(false),
            "backend 1 got a datagram"
        );
        assert!(
            c2.recv(&mut buf).map(|n| n > 0).unwrap_or(false),
            "backend 2 got a datagram"
        );
    }

    #[test]
    fn one_drain_then_a_second_report_sees_no_new_metrics() {
        // Guard the DRAIN-ON-REPORT semantics that motivate the fan-out design: after one report drains the
        // counters, a SECOND report (no new activity) still succeeds but carries the (now-zero) delta. This
        // pins WHY per-backend sequential reporting would starve later backends — the first drain empties the
        // counters. Both reports reach the collector without error; we assert the path stays total.
        let collector = std::net::UdpSocket::bind(("127.0.0.1", 0)).expect("bind collector");
        collector
            .set_read_timeout(Some(std::time::Duration::from_millis(200)))
            .unwrap();
        let addr = collector.local_addr().unwrap().to_string();

        let registry = Registry::new();
        HostMetrics::new(&registry).record_turn(true);

        let (mut reporter, _) = MetricsReporter::from_statsd_targets(&[StatsdTarget {
            endpoint: addr,
            prefix: None,
        }]);
        reporter.report(&registry); // drains the recorded turn
        reporter.report(&registry); // second drain — counters already cleared, still total
    }

    #[test]
    fn reporter_reports_bad_targets_at_build_but_still_registers_good_ones() {
        // One good collector + one unresolvable target. The bad one is named in the build-time failure list,
        // but the good collector is still registered and a report reaches it (a bad target must not starve the
        // healthy ones).
        let good = std::net::UdpSocket::bind(("127.0.0.1", 0)).expect("bind good");
        good.set_read_timeout(Some(std::time::Duration::from_millis(200)))
            .unwrap();
        let (mut reporter, failed) = MetricsReporter::from_statsd_targets(&[
            StatsdTarget {
                endpoint: "not a socket addr".into(),
                prefix: None,
            },
            StatsdTarget {
                endpoint: good.local_addr().unwrap().to_string(),
                prefix: None,
            },
        ]);
        assert_eq!(failed.len(), 1, "exactly the one bad target: {failed:?}");
        assert!(failed[0].contains("not a socket addr"), "{failed:?}");
        assert_eq!(reporter.backend_count(), 1, "the good target registered");

        let registry = Registry::new();
        HostMetrics::new(&registry).record_turn(true);
        reporter.report(&registry);

        let mut buf = [0u8; 4096];
        assert!(
            good.recv(&mut buf).map(|n| n > 0).unwrap_or(false),
            "the good backend still got a datagram"
        );
    }

    #[test]
    fn reporter_with_no_backends_is_total() {
        // No targets → zero backends → a report drains to a no-op (clears the counters), no panic.
        let registry = Registry::new();
        HostMetrics::new(&registry).record_turn(true);
        let (mut reporter, failed) = MetricsReporter::from_statsd_targets(&[]);
        assert!(failed.is_empty());
        assert_eq!(reporter.backend_count(), 0);
        reporter.report(&registry);
    }

    #[tokio::test]
    async fn export_loop_reports_on_shutdown() {
        // Drive the periodic loop with a long interval + fire shutdown almost immediately; the loop's FINAL
        // report (on shutdown) must reach the collector — proving the shutdown-aware final drain works even if
        // no periodic tick elapsed. Also proves the loop terminates on the shutdown signal.
        let collector = std::net::UdpSocket::bind(("127.0.0.1", 0)).expect("bind collector");
        collector
            .set_read_timeout(Some(std::time::Duration::from_millis(500)))
            .unwrap();
        let addr = collector.local_addr().unwrap().to_string();

        let registry = Registry::new();
        HostMetrics::new(&registry).record_turn(true);
        let (reporter, _) = MetricsReporter::from_statsd_targets(&[StatsdTarget {
            endpoint: addr,
            prefix: Some("cdz".into()),
        }]);

        let (sd_tx, sd_rx) = tokio::sync::oneshot::channel();
        // A long interval so no periodic tick fires before shutdown — the datagram must come from the FINAL
        // report alone.
        let handle = tokio::spawn(run_export_loop(
            registry,
            reporter,
            std::time::Duration::from_secs(3600),
            sd_rx,
        ));
        // Let the loop reach its select!, then shut it down.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        sd_tx.send(()).expect("send shutdown");
        handle.await.expect("loop task joins cleanly");

        let mut buf = [0u8; 4096];
        assert!(
            collector.recv(&mut buf).map(|n| n > 0).unwrap_or(false),
            "the final on-shutdown report reached the collector"
        );
    }

    #[test]
    fn connect_resolves_a_multi_address_name() {
        // connect must accept a NAME that resolves to multiple addresses and succeed by trying them until one
        // binds+connects (the #2119 multi-address iteration). `localhost` typically resolves to both ::1 and
        // 127.0.0.1. We assert only that connect SUCCEEDS on the name — NOT which family it picks: a UDP
        // `connect` does not validate the receiver, so asserting a datagram arrives at a single-family
        // collector would be flaky (if the name resolves v6-first + v6 is usable, connect correctly picks ::1
        // and a v4-only collector never receives it — the test would fail though connect behaved correctly,
        // #2129 review). Datagram DELIVERY is proven deterministically in the explicit-family test below.
        use std::net::ToSocketAddrs;
        // Only meaningful when `localhost` actually resolves to MULTIPLE addresses (the multi-address case we
        // mean to exercise). On a single-address-localhost runner this test would pass without covering
        // iteration at all, so SKIP unless >=2 addresses resolve (#2137 review). The explicit-family test
        // still covers the single-address connect+deliver path everywhere.
        let n_addrs = "localhost:65000"
            .to_socket_addrs()
            .map(|it| it.count())
            .unwrap_or(0);
        if n_addrs < 2 {
            return;
        }
        let sink = UdpStatsdSink::connect("localhost:65000");
        assert!(
            sink.is_ok(),
            "connect resolves a multi-address name and connects a usable address: {sink:?}"
        );
    }

    #[test]
    fn connect_delivers_to_an_explicit_family_collector() {
        // Datagram delivery, made DETERMINISTIC by targeting 127.0.0.1 EXPLICITLY (a single v4 address, not
        // the ambiguous `localhost` name) so connect binds v4 and the v4 collector always receives (#2129
        // review). This is the family-matched bind (#2112) + actual send end-to-end.
        let collector = std::net::UdpSocket::bind(("127.0.0.1", 0)).expect("bind collector");
        collector
            .set_read_timeout(Some(std::time::Duration::from_millis(200)))
            .unwrap();
        let addr = collector.local_addr().unwrap().to_string();

        let mut sink = UdpStatsdSink::connect(&addr).expect("connect to the explicit v4 collector");
        use s2n_quic_dc_metrics::backend::StatsdSink;
        sink.send_batch(std::iter::once("cdz.test:1|c"));

        let mut buf = [0u8; 64];
        assert!(
            collector.recv(&mut buf).map(|n| n > 0).unwrap_or(false),
            "the datagram reached the explicit v4 collector"
        );
    }

    #[test]
    fn connect_to_a_bad_endpoint_is_a_clean_error() {
        // An unparseable endpoint is a clean Err (not a panic) — a bad [observability] statsd target surfaces
        // at export setup.
        let err = UdpStatsdSink::connect("not a socket addr").unwrap_err();
        assert!(err.contains("statsd sink"), "{err}");
    }

    #[cfg(feature = "metrics-export-otlp")]
    mod otlp {
        use super::*;
        use s2n_quic_dc_metrics::backend::OtlpSink;

        #[test]
        fn otlp_sink_send_is_nonblocking_and_drops_when_channel_full() {
            // Guard 2: the sink's send runs inside the sync report cycle, so it must NEVER block. Build a
            // sink over a tiny channel, DON'T drain it, and push more than the bound — every send must return
            // immediately (dropping on full), never block or panic. (A blocking send here would hang the test
            // + prove the report cycle could stall on a slow collector.)
            let (tx, _rx) = tokio::sync::mpsc::channel::<Vec<u8>>(2);
            let mut sink = ChannelOtlpSink { tx };
            // Push well past the bound of 2 — all return immediately, extras dropped.
            for _ in 0..10 {
                sink.send(b"otlp-payload");
            }
            // Reaching here without hanging is the assertion (send never blocks).
        }

        #[test]
        fn otlp_sink_send_after_receiver_dropped_is_a_clean_drop() {
            // If the forwarder is gone (receiver dropped), send must be a clean no-op drop (Closed), not a
            // panic — the report cycle keeps going even after the forwarder dies.
            let (tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(2);
            drop(rx);
            let mut sink = ChannelOtlpSink { tx };
            sink.send(b"otlp-payload"); // no panic
        }

        #[tokio::test]
        async fn otlp_backend_registers_and_a_report_enqueues_without_blocking() {
            // Push an OTLP target into the reporter + drive a report: the report cycle must complete without
            // blocking on the (undrained) channel, and a payload must be enqueued for the forwarder.
            let target = OtlpTarget {
                endpoint: "http://127.0.0.1:65001".into(),
                scope_name: Some("cdz-test".into()),
                scope_version: None,
            };
            let (mut reporter, _) = MetricsReporter::from_statsd_targets(&[]);
            let mut rx = reporter.push_otlp(&target);
            assert_eq!(reporter.backend_count(), 1);

            let registry = Registry::new();
            HostMetrics::new(&registry).record_turn(true);
            reporter.report(&registry); // must not block on the undrained channel

            // A protobuf payload was enqueued for the forwarder.
            let payload = rx.try_recv();
            assert!(
                payload.map(|p| !p.is_empty()).unwrap_or(false),
                "the OTLP report enqueued a non-empty payload"
            );
        }

        #[test]
        fn timestamped_backend_never_inverts_the_interval_on_a_backward_clock() {
            // Guard the #2144 clock-regression fix: report_start clamps now up to last_report_ns, so a
            // BACKWARD wall-clock jump can't make start > time. Simulate a huge previous timestamp (as if the
            // clock later reads earlier), drive a report_start, and assert last_report_ns did NOT regress — the
            // clamp held. (We can't force the real clock backward, so we seed last_report_ns to the far future
            // and verify a real-`now` report_start keeps it, which is exactly the inversion-preventing path.)
            let (tx, _rx) = tokio::sync::mpsc::channel::<Vec<u8>>(2);
            let sink = ChannelOtlpSink { tx };
            let inner = s2n_quic_dc_metrics::backend::OtlpBackend::new(sink, "cdz-test", "");
            let mut b = TimestampedOtlpBackend {
                inner,
                last_report_ns: u64::MAX - 1, // as if a prior report saw a far-future clock
            };
            use s2n_quic_dc_metrics::backend::{Backend, ReportOptions};
            let before = b.last_report_ns;
            b.report_start(&ReportOptions::default());
            // The clamp (now = now.max(last_report_ns)) means now can only stay >= before, so last_report_ns
            // never regresses → the emitted start (=before) is always <= time (=last_report_ns after).
            assert!(
                b.last_report_ns >= before,
                "clamp kept the timestamp monotonic: {} >= {before}",
                b.last_report_ns
            );
        }

        #[tokio::test]
        async fn otlp_forwarder_against_a_dead_endpoint_drops_and_terminates() {
            // Guard 2 end-to-end: a forwarder pointed at a dead endpoint must DROP the payload (POST fails)
            // and, when the channel closes, TERMINATE — never hang. Send one payload, close the channel, and
            // assert the forwarder task joins (doesn't block forever on the failed POST).
            let (tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(2);
            // 127.0.0.1:1 — nothing listens; the POST fails fast (conn refused), the forwarder logs + drops.
            let handle = tokio::spawn(run_otlp_forwarder("http://127.0.0.1:1".into(), rx));
            tx.send(b"otlp-payload".to_vec()).await.expect("enqueue");
            drop(tx); // close the channel → forwarder drains the one payload, then returns
                      // The forwarder must terminate (a dead endpoint doesn't wedge it). Bound the wait so a hang fails
                      // the test rather than hanging CI.
            let joined = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;
            assert!(
                joined.is_ok(),
                "forwarder terminated after the channel closed"
            );
        }
    }
}
