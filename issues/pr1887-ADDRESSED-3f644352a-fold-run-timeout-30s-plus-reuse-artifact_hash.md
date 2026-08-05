# PR #1887 review comments — cdz-agent-host/tests/name_store_publish_consume_e2e.rs (v-agent-harness-host) — OPEN

https://github.com/camshaft/cadenza/pull/1887 (the fix for my #1883 — publish→consume demo now RUNS a fold).
The fold-running I asked for introduced an unbounded deliver.

## 1. `HostedSession::deliver` drives to quiescence with NO step-bound → a misbehaving live reducer hangs the suite (Copilot, name_store_publish_consume_e2e.rs:185) — test-robustness [same class as #1853]
> `deliver` drives the reducer/effect loop to quiescence with no built-in step bound; enabled via
> `CDZ_LIVE_REDUCER_COMPONENT`, a misbehaving/changed live reducer could loop indefinitely and hang the
> test suite. Wrap the `deliver(..)` future in `tokio::time::timeout(..)`.
Ironic follow-on: #1883 asked this demo to actually RUN the fold — and running it (via HostedSession::
deliver, unbounded) reintroduces the exact hang-risk the #1853 fix addressed for the other live calls.
Wrap deliver(..) in tokio::time::timeout(LIVE_CALL_TIMEOUT or similar) so a looping reducer surfaces as a
bounded error, not an infinite stall. LOW-MED/test-robustness (live-gated, but a hang is worse than a
fail). Fix-forward. (Same timeout discipline as #1853/#1857.)

## 2. `artifact_hash` re-hashes bytes that are already content-addressed (Copilot, :177) — cleanliness
> `artifact_hash` is already the content address for `fetched` (from `blobs.put(&component)`, and you
> assert `fetched == component`). Re-hashing the full wasm bytes is redundant + obscures that the reducer
> id matches the published pointer's resolved hash.
Reuse the existing content address (the put/resolve hash) instead of re-hashing the full bytes — cheaper +
clearer that the reducer id == the published pointer's resolved hash. LOW/cleanliness.
