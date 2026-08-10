# PR #2144 review — cdz-agent-host/src/export.rs (v-agent-harness-host) — OPEN — 2 MED + 1 LOW [VERIFIED]

https://github.com/camshaft/cadenza/pull/2144 (OTLP metrics-export backend — sync-send via bounded
channel + async HTTP forwarder, metrics-export-otlp feature). Copilot 3 inline.

## `run_otlp_forwarder` builds `reqwest::Client::new()` with NO connect/request timeout → a blackholed collector hangs `.send().await` indefinitely, defeating the channel-close→terminate contract + making the dead-endpoint test flaky (Copilot, export.rs:328) — reliability [VERIFIED, MED]
> `run_otlp_forwarder` issues reqwest POSTs with no connect/request timeout. If the collector endpoint
> blackholes connections (e.g., firewall drop), the forwarder task can hang indefinitely on
> `.send().await`, preventing the "channel closes → forwarder terminates" behavior and making the
> `otlp_forwarder_against_a_dead_endpoint_*` test potentially flaky.

VERIFIED in the #2144 diff: `let client = reqwest::Client::new();` (diff:348) — the default client has NO
connect or request timeout. The forwarder loop then does `client.post(url)...send().await` (diff:350-354)
inside `while let Some(payload) = rx.recv().await`. If the endpoint blackholes (firewall DROP, not RST),
the connect blocks with no deadline, so the task is stuck IN `.send().await` and never returns to `recv()`
— so a subsequent channel-close can't terminate it (the terminate-on-close contract only checks between
iterations). The PR's own `otlp_forwarder_against_a_dead_endpoint_drops_and_terminates` test (diff:457)
depends on the forwarder actually terminating; against a true blackhole it'd hang until the outer 5s
`tokio::time::timeout` (diff:468) fires — flaky/slow. MED. Fix per Copilot: build the client with bounded
`.connect_timeout(...)` + `.timeout(...)`, and exit cleanly if `Client::builder().build()` errors (log +
return, don't unwrap). This makes both the blackhole path and the test deterministic.

## `report_start` can set `start_time_ns` AFTER `time_ns` on a backwards clock jump / `now_unix_ns()`-returns-0-on-error → OTLP requires start <= time; an inverted interval breaks collector ingestion (Copilot, export.rs:280) — correctness [VERIFIED, MED]
> `TimestampedOtlpBackend::report_start` can set `start_time_ns` after `time_ns` if the system clock
> jumps backwards (or if `now_unix_ns()` returns 0 on error while `last_report_ns` is non-zero). OTLP
> datapoints require start <= time; emitting an inverted interval can break collector
> ingestion/aggregation. Clamp `now` to be monotonic non-decreasing relative to `last_report_ns`.

VERIFIED in the diff: `report_start` (diff:295-306) sets `start = if last_report_ns == 0 { now } else {
last_report_ns }`, then `set_time_ns(now)` and `last_report_ns = now`. The first-report case is guarded
(start==now, zero-length, diff:297-299 comment). But on report N>1: `start = last_report_ns` (the PREVIOUS
now) and `time = now` (current). If the wall clock jumped BACKWARDS between reports, `now < last_report_ns`
→ `start > time` — inverted interval. Also `now_unix_ns()` does `.unwrap_or(0)` on a SystemTime error
(diff:285-291): if it returns 0 while `last_report_ns` is non-zero, `start = last_report_ns > 0 = time` —
same inversion. OTLP datapoints require `start_time_unix_nano <= time_unix_nano`; an inverted interval
breaks collector ingestion/aggregation. MED. Fix per Copilot: clamp `now` to be monotone non-decreasing
vs `last_report_ns` (`let now = max(now, last_report_ns)`), so start <= time always holds. (Ties into the
`now_unix_ns` unwrap_or(0) — the clamp also absorbs the error-returns-0 case.)

## module doc for `TimestampedOtlpBackend` says the first interval uses "process start", but the impl sets `start == now` (zero-length) for the first report → doc/behavior mismatch (Copilot, export.rs:248) — doc-accuracy [VERIFIED, LOW]
> The doc comment for `TimestampedOtlpBackend` says the first interval uses "process start", but the
> implementation sets `start == now` for the first report (zero-length interval). This mismatch can
> mislead future changes/debugging; update the comment (or store process start explicitly if desired).
VERIFIED: the doc (diff:274) says "`start` is the previous report's end (process start for [the first])",
but the impl (diff:297-299) sets `start = now` for the first report with the comment "First report: start
== now (a zero-length interval)". So the doc claims process-start while the code uses now. LOW/doc-accuracy.
Fix: reword the doc to say the first interval is zero-length (start==now), OR store process-start explicitly
if that's the intended semantics. (The two MED are the ones that matter; the timeout especially.)
v-agent-harness-host owns cdz-agent-host/src. PR OPEN → all three foldable pre-merge.
