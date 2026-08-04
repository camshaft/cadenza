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
    fn connect_tries_all_resolved_addresses() {
        // `localhost` typically resolves to BOTH ::1 and 127.0.0.1 (in some order). connect must succeed by
        // trying addresses until one binds+connects — proving the multi-address iteration (#2119 review): even
        // if the first-resolved family were unusable, a later address still connects. We bind a v4 collector
        // and target it by the `localhost:port` NAME so resolution yields multiple candidates.
        let collector = std::net::UdpSocket::bind(("127.0.0.1", 0)).expect("bind collector");
        collector
            .set_read_timeout(Some(std::time::Duration::from_millis(200)))
            .unwrap();
        let port = collector.local_addr().unwrap().port();

        // Skip cleanly if `localhost` doesn't resolve to a v4 address on this runner (unusual).
        use std::net::ToSocketAddrs;
        let has_v4 = format!("localhost:{port}")
            .to_socket_addrs()
            .map(|it| it.into_iter().any(|a| a.is_ipv4()))
            .unwrap_or(false);
        if !has_v4 {
            return;
        }

        let sink = UdpStatsdSink::connect(&format!("localhost:{port}"))
            .expect("connect resolves localhost and connects a usable address");
        // Prove the connected socket actually reaches the v4 collector.
        use s2n_quic_dc_metrics::backend::StatsdSink;
        let mut sink = sink;
        sink.send_batch(std::iter::once("cdz.test:1|c"));
        let mut buf = [0u8; 64];
        // The datagram lands only if connect picked (or fell back to) the v4 address the collector is on.
        assert!(
            collector.recv(&mut buf).map(|n| n > 0).unwrap_or(false),
            "a datagram reached the v4 collector via a resolved address"
        );
    }

    #[test]
    fn connect_to_a_bad_endpoint_is_a_clean_error() {
        // An unparseable endpoint is a clean Err (not a panic) — a bad [observability] statsd target surfaces
        // at export setup.
        let err = UdpStatsdSink::connect("not a socket addr").unwrap_err();
        assert!(err.contains("statsd sink"), "{err}");
    }
}
