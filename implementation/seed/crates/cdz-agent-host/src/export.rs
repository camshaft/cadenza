//! Metrics EXPORT — the emitter that periodically reports the daemon's metrics [`Registry`] to a backend
//! (statsd first) over the network. Behind the `metrics-export` cargo feature (OFF by default): the metric
//! RECORDING (the registry Counters/Summaries in [`crate::metrics`]) is always-on + hermetic, but the
//! EXPORT — sending to a collector — is gated so the default build never opens a socket / sends UDP.
//!
//! The registry Counters are DRAIN-ON-REPORT: each [`Registry::report`] emits the per-interval delta and
//! clears the counters, so the report cadence (`[observability].report_interval_secs`) IS the aggregation
//! window — exactly the statsd/otlp PUSH model (the collector does the time-series aggregation). The intended
//! caller is the daemon's periodic export task: it will run [`report_once`] on a `report_interval_secs` timer
//! per configured target (that timer-loop wiring is a following slice — this module provides the flush
//! primitive it drives).
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
        // Resolve the endpoint FIRST so the ephemeral local socket can be bound in the MATCHING address
        // family: a v4-bound socket (`0.0.0.0:0`) cannot `connect` to an IPv6 peer (EAFNOSUPPORT), so a
        // statsd collector that resolves to / is configured as IPv6 would fail even when reachable over v6
        // (#2112 review). Bind `[::]:0` for a v6 target, `0.0.0.0:0` for v4, then connect so subsequent
        // `send` (no addr) routes to the collector.
        let target = endpoint
            .to_socket_addrs()
            .map_err(|e| format!("statsd sink could not resolve {endpoint}: {e}"))?
            .next()
            .ok_or_else(|| format!("statsd sink: {endpoint} resolved to no address"))?;
        let bind_addr: std::net::SocketAddr = if target.is_ipv6() {
            (std::net::Ipv6Addr::UNSPECIFIED, 0).into()
        } else {
            (std::net::Ipv4Addr::UNSPECIFIED, 0).into()
        };
        let socket = std::net::UdpSocket::bind(bind_addr)
            .map_err(|e| format!("statsd sink could not bind a local UDP socket: {e}"))?;
        socket
            .connect(target)
            .map_err(|e| format!("statsd sink could not connect to {endpoint}: {e}"))?;
        Ok(UdpStatsdSink { socket })
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
    fn connect_to_a_bad_endpoint_is_a_clean_error() {
        // An unparseable endpoint is a clean Err (not a panic) — a bad [observability] statsd target surfaces
        // at export setup.
        let err = UdpStatsdSink::connect("not a socket addr").unwrap_err();
        assert!(err.contains("statsd sink"), "{err}");
    }
}
