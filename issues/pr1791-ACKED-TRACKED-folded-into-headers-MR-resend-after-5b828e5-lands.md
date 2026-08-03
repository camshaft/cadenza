# PR #1791 review comment — cdz-agent-host/src/http.rs (v-agent-harness-host) — OPEN

https://github.com/camshaft/cadenza/pull/1791 (HTTP request headers + response status).

## HttpTransport doc mixes 4xx/5xx status into `Err(reason)` retryability guidance, but completed requests with a status aren't `Err` (Copilot, http.rs:100) — doc/consistency
> The HttpTransport doc mixes HTTP status codes (4xx/5xx) into the `Err(reason)` retryability guidance, but
> immediately below it states that completed requests WITH a status are [returned as Ok, not Err].
The doc conflates transport-level `Err(reason)` (connection/timeout — retryable) with application-level
4xx/5xx statuses (which come back as a completed Ok response, not Err). Separate the two in the doc: Err =
transport failure retryability; a 4xx/5xx is a successful transport with a status the caller interprets.
LOW/doc-consistency. Fix-forward.
