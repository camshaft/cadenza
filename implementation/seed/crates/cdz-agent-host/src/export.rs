//! Metrics EXPORT — the emitter that periodically reports the daemon's metrics [`Registry`] to a backend
//! (statsd first) over the network. Behind the `metrics-export` cargo feature (OFF by default): the metric
//! RECORDING (the registry Counters/Summaries in [`crate::metrics`]) is always-on + hermetic, but the
//! EXPORT — sending to a collector — is gated so the default build never opens a socket / sends UDP.
//!
//! The registry Counters are DRAIN-ON-REPORT: each [`Registry::report`] emits the per-interval delta and
//! clears the counters, so the report cadence (`[observability].report_interval_secs`) IS the aggregation
//! window — exactly the statsd/otlp PUSH model (the collector does the time-series aggregation).
//!
//! Three layers: [`report_once`] flushes to ONE endpoint; [`report_statsd_once`] flushes to MANY statsd
//! targets with a SINGLE registry drain fanned out (multi-target-correct — a per-target report loop would
//! drain on the first and hand zeroes to the rest); [`run_statsd_export_loop`] is the daemon's periodic,
//! shutdown-aware task that calls [`report_statsd_once`] every `report_interval_secs` and does one final
//! flush on shutdown. The daemon spawns the loop with a clone of the host's registry.
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

/// Report the current metrics from `registry` to the statsd collector at `endpoint` ONCE (a single flush).
/// Builds a fresh [`StatsdBackend`](s2n_quic_dc_metrics::backend::StatsdBackend) over a UDP sink to
/// `endpoint` + drives [`Registry::report`] into it — draining the per-interval deltas to the collector.
/// `prefix` optionally namespaces every metric name (e.g. `"cdz.agent"`). Borrowed (`Option<&str>`) so the
/// periodic-flush caller passes its configured prefix each tick WITHOUT cloning; the one owned `String` the
/// backend needs is allocated inside (#2112 review). `Err` only if the sink can't be set up (bad endpoint); a
/// per-datagram send failure is swallowed inside the sink (best-effort).
///
/// The daemon calls this on each tick of its `report_interval_secs` timer, per configured statsd target.
pub fn report_once(
    registry: &Registry,
    endpoint: &str,
    prefix: Option<&str>,
) -> Result<(), String> {
    use s2n_quic_dc_metrics::backend::StatsdBackend;
    let sink = UdpStatsdSink::connect(endpoint)?;
    let mut backend = StatsdBackend::new(sink, prefix.map(str::to_owned));
    registry.report(&mut backend);
    Ok(())
}

/// One resolved statsd export target the periodic loop flushes to — where to send + an optional metric-name
/// prefix. OWNED (not borrowed) so the loop's spawned task holds its target set for the daemon's lifetime
/// without a config borrow. The daemon builds these from the `statsd` entries of `[observability].target`.
#[derive(Debug, Clone)]
pub struct StatsdTarget {
    /// The statsd collector address (`host:port`) UDP payloads go to.
    pub endpoint: String,
    /// Optional metric-name prefix (maps to `StatsdBackend`'s prefix), e.g. `"cdz.agent"`.
    pub prefix: Option<String>,
}

/// Flush the current metrics from `registry` to EVERY statsd target ONCE, draining the registry a SINGLE
/// time and fanning the drained values out to all targets. This is the multi-target-correct primitive: the
/// registry Counters are DRAIN-ON-REPORT, so reporting per-target in a loop would drain on the first target
/// and hand ZEROES to every target after it (#2119 follow-on) — instead we build one
/// [`StatsdBackend`](s2n_quic_dc_metrics::backend::StatsdBackend) per target over a
/// [`Vec<Backend>`](s2n_quic_dc_metrics::backend) (the crate's fan-out impl) and [`Registry::report`] into it
/// once, so every target sees the same per-interval delta with its OWN prefix applied.
///
/// Best-effort: a target whose sink can't connect this tick (collector down, bad addr) is SKIPPED and its
/// endpoint collected into the returned error list, but the registry is STILL drained to the targets that DID
/// connect (a transient bad target must not stall export to the healthy ones, nor wedge the counters
/// un-drained). `Ok(())` when all targets connected; `Err(list)` names the ones that didn't (the caller — the
/// periodic loop — logs them and carries on). An empty `targets` slice still drains to a no-op backend
/// (clears the counters) and is `Ok`.
pub fn report_statsd_once(
    registry: &Registry,
    targets: &[StatsdTarget],
) -> Result<(), Vec<String>> {
    use s2n_quic_dc_metrics::backend::StatsdBackend;

    let mut backends: Vec<StatsdBackend<UdpStatsdSink>> = Vec::with_capacity(targets.len());
    let mut failed = Vec::new();
    for t in targets {
        match UdpStatsdSink::connect(&t.endpoint) {
            Ok(sink) => backends.push(StatsdBackend::new(sink, t.prefix.clone())),
            Err(e) => failed.push(format!("{}: {e}", t.endpoint)),
        }
    }
    // Drain the registry EXACTLY ONCE, fanning out to all connected backends (Vec<B>: Backend). Even an empty
    // Vec drains (to a no-op), which correctly clears the per-interval counters.
    registry.report(&mut backends);

    if failed.is_empty() {
        Ok(())
    } else {
        Err(failed)
    }
}

/// Run the daemon's periodic metrics-export loop: every `interval`, flush the metrics [`Registry`] to all
/// `targets` via [`report_statsd_once`] (a single drain fanned out), until `shutdown` fires — then do ONE
/// FINAL flush (so the last partial interval's metrics aren't lost) and return. Per-tick connect failures are
/// LOGGED (a `tracing` warn) and swallowed — export is best-effort and must never take the daemon down.
///
/// `registry` is a CLONE of the host's registry (the `Arc`-backed handle shares the same storage), so the
/// loop reads the same counters the host loop increments. The task is spawned by the daemon and shut down
/// alongside the host loop.
pub async fn run_statsd_export_loop(
    registry: Registry,
    targets: Vec<StatsdTarget>,
    interval: std::time::Duration,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
) {
    let mut timer = tokio::time::interval(interval);
    // Skip the immediate first tick's burst-catch-up semantics: we want steady cadence, and a flush right at
    // boot (before any work) would just emit zeroes.
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Consume the interval's immediate first tick so the first REAL flush happens one `interval` in.
    timer.tick().await;

    loop {
        tokio::select! {
            _ = timer.tick() => {
                if let Err(failed) = report_statsd_once(&registry, &targets) {
                    tracing::warn!(targets = ?failed, "metrics-export: statsd target(s) unreachable this interval");
                }
            }
            _ = &mut shutdown => {
                // Final drain on the way out so the last interval's metrics reach the collector.
                if let Err(failed) = report_statsd_once(&registry, &targets) {
                    tracing::warn!(targets = ?failed, "metrics-export: statsd target(s) unreachable on final flush");
                }
                tracing::debug!("metrics-export: statsd export loop shut down");
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
    fn report_once_sends_to_a_bound_collector_without_error() {
        // Bind a real UDP socket as the "collector", record some metrics into a registry, then report_once
        // to that socket's addr — proves the sink connects + the registry drives the StatsdBackend + a
        // datagram is sent, all without error. (We don't assert the payload bytes — statsd wire format is the
        // crate's concern; this proves the export path is wired + total.)
        let collector = std::net::UdpSocket::bind(("127.0.0.1", 0)).expect("bind collector");
        collector
            .set_read_timeout(Some(std::time::Duration::from_millis(200)))
            .unwrap();
        let addr = collector.local_addr().unwrap().to_string();

        let registry = Registry::new();
        let host = HostMetrics::new(&registry);
        host.record_session_installed();
        host.record_turn(true);

        report_once(&registry, &addr, Some("cdz.agent")).expect("report_once ok");

        // A datagram arrived at the collector (the statsd export actually sent something).
        let mut buf = [0u8; 4096];
        let got = collector.recv(&mut buf);
        assert!(got.is_ok(), "collector received a statsd datagram: {got:?}");
        assert!(got.unwrap() > 0, "the datagram was non-empty");
    }

    #[test]
    fn report_once_reaches_an_ipv6_collector() {
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
        let host = HostMetrics::new(&registry);
        host.record_turn(true);

        report_once(&registry, &addr, None).expect("report_once to a v6 collector ok");

        let mut buf = [0u8; 4096];
        assert!(
            collector.recv(&mut buf).is_ok(),
            "the v6 collector received a statsd datagram"
        );
    }

    #[test]
    fn report_statsd_once_fans_one_drain_out_to_every_target() {
        // Two collectors, both configured as targets. report_statsd_once drains the registry ONCE and fans
        // out — so BOTH collectors receive a (non-empty) datagram from the SAME drain. This is the
        // multi-target-correctness guard: a per-target report loop would drain on the first + send zeroes (or
        // nothing) to the second.
        let c1 = std::net::UdpSocket::bind(("127.0.0.1", 0)).expect("bind c1");
        let c2 = std::net::UdpSocket::bind(("127.0.0.1", 0)).expect("bind c2");
        for c in [&c1, &c2] {
            c.set_read_timeout(Some(std::time::Duration::from_millis(200)))
                .unwrap();
        }
        let targets = vec![
            StatsdTarget {
                endpoint: c1.local_addr().unwrap().to_string(),
                prefix: Some("a".into()),
            },
            StatsdTarget {
                endpoint: c2.local_addr().unwrap().to_string(),
                prefix: Some("b".into()),
            },
        ];

        let registry = Registry::new();
        let host = HostMetrics::new(&registry);
        host.record_session_installed();
        host.record_turn(true);

        report_statsd_once(&registry, &targets).expect("all targets connected");

        let mut buf = [0u8; 4096];
        assert!(
            c1.recv(&mut buf).map(|n| n > 0).unwrap_or(false),
            "target 1 got a datagram"
        );
        assert!(
            c2.recv(&mut buf).map(|n| n > 0).unwrap_or(false),
            "target 2 got a datagram"
        );
    }

    #[test]
    fn report_statsd_once_reports_bad_targets_but_still_drains_good_ones() {
        // One good collector + one unresolvable target. The bad one is named in the Err list, but the good
        // collector STILL receives its datagram (a transient bad target must not starve the healthy ones).
        let good = std::net::UdpSocket::bind(("127.0.0.1", 0)).expect("bind good");
        good.set_read_timeout(Some(std::time::Duration::from_millis(200)))
            .unwrap();
        let targets = vec![
            StatsdTarget {
                endpoint: "not a socket addr".into(),
                prefix: None,
            },
            StatsdTarget {
                endpoint: good.local_addr().unwrap().to_string(),
                prefix: None,
            },
        ];

        let registry = Registry::new();
        HostMetrics::new(&registry).record_turn(true);

        let err = report_statsd_once(&registry, &targets).expect_err("one target is bad");
        assert_eq!(
            err.len(),
            1,
            "exactly the one bad target is reported: {err:?}"
        );
        assert!(err[0].contains("not a socket addr"), "{err:?}");

        let mut buf = [0u8; 4096];
        assert!(
            good.recv(&mut buf).map(|n| n > 0).unwrap_or(false),
            "the good target still got a datagram"
        );
    }

    #[test]
    fn report_statsd_once_with_no_targets_is_ok() {
        // No targets → drains to a no-op backend (clears the counters), no error.
        let registry = Registry::new();
        HostMetrics::new(&registry).record_turn(true);
        report_statsd_once(&registry, &[]).expect("empty target set is Ok");
    }

    #[tokio::test]
    async fn export_loop_flushes_on_shutdown() {
        // Drive the periodic loop with a short interval + fire shutdown almost immediately; the loop's FINAL
        // flush (on shutdown) must reach the collector — proving the shutdown-aware final drain works even if
        // no periodic tick elapsed. Also proves the loop terminates on the shutdown signal.
        let collector = std::net::UdpSocket::bind(("127.0.0.1", 0)).expect("bind collector");
        collector
            .set_read_timeout(Some(std::time::Duration::from_millis(500)))
            .unwrap();
        let addr = collector.local_addr().unwrap().to_string();

        let registry = Registry::new();
        HostMetrics::new(&registry).record_turn(true);
        let targets = vec![StatsdTarget {
            endpoint: addr,
            prefix: Some("cdz".into()),
        }];

        let (sd_tx, sd_rx) = tokio::sync::oneshot::channel();
        // A long interval so no periodic tick fires before shutdown — the datagram must come from the FINAL
        // flush alone.
        let handle = tokio::spawn(run_statsd_export_loop(
            registry,
            targets,
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
            "the final on-shutdown flush reached the collector"
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
