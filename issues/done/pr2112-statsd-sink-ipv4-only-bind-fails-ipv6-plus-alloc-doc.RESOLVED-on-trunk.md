# PR #2112 review — cdz-agent-host/src/export.rs (v-agent-harness-host) — OPEN — 1 MED portability + 2 LOW (batched)

https://github.com/camshaft/cadenza/pull/2112 (metrics EXPORT emitter — statsd UDP sink). Copilot 3 inline.

## `UdpStatsdSink::connect` binds an IPv4-only local socket (`0.0.0.0:0`) then `connect(endpoint)` → fails for an IPv6 endpoint even when reachable (Copilot, export.rs:40) — portability [VERIFIED, MED]
> `UdpStatsdSink::connect` binds an IPv4-only socket ("0.0.0.0:0") and then calls `connect(endpoint)`. This
> will fail for IPv6 endpoints (address family mismatch) even if the host can reach the collector.
> Consider resolving the endpoint first and binding an ephemeral socket with the matching address family
> (v4 vs v6) before connecting.

VERIFIED in the #2112 diff: `let socket = std::net::UdpSocket::bind(("0.0.0.0", 0))?;` (diff:59 — IPv4
wildcard) then `socket.connect(endpoint)?` (diff:62). A UDP socket bound to an IPv4 local address cannot
`connect` to an IPv6 peer → `EAFNOSUPPORT`/address-family mismatch. So a statsd collector that resolves to
(or is configured as) an IPv6 address fails to connect even when reachable over v6. MED/portability
(bites v6-only or v6-first-resolving collectors). Fix per Copilot: resolve `endpoint` via `to_socket_addrs`
first, pick the resolved addr, and bind an ephemeral socket of the MATCHING family (`0.0.0.0:0` for a v4
peer, `[::]:0` for a v6 peer) before `connect`. (Same address-hardening lineage as the SEC-F1 URL/host
work — network endpoints shouldn't assume v4.)

## `report_once` takes `prefix: Option<String>` → per-tick alloc/clone on a repeatedly-called fn (Copilot, export.rs:69) — efficiency [VERIFIED, LOW]
> `report_once` takes `prefix: Option<String>`, which forces callers to allocate/clone the prefix on every
> periodic tick … Consider taking `Option<&str>` and only allocating inside `report_once`.
VERIFIED — `report_once` is the per-interval flush; an owned `Option<String>` param makes each tick's
caller clone the prefix. LOW/efficiency. Fix: `Option<&str>`, allocate only when constructing the backend
name inside. (Minor — the tick interval is seconds-scale — but free.)

## module doc says "The daemon runs `report_once`" but the daemon doesn't call this module yet (export unwired, per config.rs) (Copilot, export.rs:9) — doc-accuracy [VERIFIED, LOW]
> The module docs currently state "The daemon runs `report_once`…" but `cdz_agent_daemon` does not call
> into this module yet (and config.rs explicitly notes export is not wired). … word it as an
> intended/typical caller rather than current behavior.
VERIFIED — same config-ahead-of-wiring staging class (cf #1981/#2076/#2105). Reword to "the daemon WILL
run report_once (wiring is a following slice)" / "a typical caller runs…". LOW. v-agent-harness-host owns
cdz-agent-host/src. The IPv6 bind is the one that matters.
