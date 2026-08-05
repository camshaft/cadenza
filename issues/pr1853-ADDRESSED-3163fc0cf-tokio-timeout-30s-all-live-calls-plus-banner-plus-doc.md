# PR #1853 review comments — cdz-agent-host/tests/live_transport_e2e.rs (v-agent-harness-host) — OPEN

https://github.com/camshaft/cadenza/pull/1853 (env-gated live integration tests).

## 1+4. Live-net tests can HANG indefinitely — no request timeout (Copilot, :39 + :112) — test-robustness [substantive]
> These live-network tests can hang indefinitely if the endpoint stalls (reqwest has no default overall
> request timeout). [Bedrock invocations similarly unbounded.] Wrap in tokio::time::timeout.
An opt-in live-net run that WEDGES on a stalled endpoint/SDK is a real footgun (a hung CI/nightly job).
Wrap both the HTTP GET (:39) and the Bedrock invoke (:112) in a `tokio::time::timeout` (with a clear
timeout-exceeded failure). LOW-MED/test-robustness — these are opt-in, but a hang is worse than a fail.

## 2. Skip banner names the wrong test fn (Copilot, :30) — doc
> The skip banner names `a_real_http_get`, but the fn is `a_real_http_get_returns_a_live_response`.
Fix the banner to the actual fn name for greppability. LOW.

## 3. Doc says skips unless model-id AND creds/region present, but code only skips on model-id (Copilot, :93) — correctness/test
> The doc implies skip unless model-id AND AWS creds/region; the code only skips on model-id — missing
> creds/region would let the test run and fail (not skip).
Either also skip when creds/region are absent (match the doc) or fix the doc to say only model-id gates the
skip. LOW-MED (a missing-creds run fails instead of skipping — noisy for opt-in). Fix-forward.
