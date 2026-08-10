# PR #1987 review — cdz-agent-host/src/factory.rs (v-agent-harness-host) — MERGED — availability/perf [VERIFIED]

https://github.com/camshaft/cadenza/pull/1987 (wire the ComponentSessionFactory into the daemon — the
factory-wiring follow-up that resolves my #1977 install-doc + #1981 blob-config staging). Copilot (id
3710713287) flags the live executor set is rebuilt on every install, inside the single-threaded loop.

## `LiveExecutorSet::build` calls `live_executor_set().await` PER INSTALL — rebuilding the reqwest client + reloading AWS defaults (Bedrock/IMDS probe) inside the single-threaded host loop → a slow probe stalls ALL session processing (Copilot, factory.rs:68 & :70) — availability [VERIFIED]
> `LiveExecutorSet::build` calls `live_executor_set().await` for every install. That rebuilds the reqwest
> client and reloads AWS defaults (`BedrockModelTransport::new`), and it happens inside the
> single-threaded host loop's admin handler, so a slow config/IMDS probe can stall all session processing
> during installs. Consider building the live transports/config once at daemon startup and having the
> builder clone shared handles … into a fresh `CompositeExecutor` per session.

VERIFIED on trunk. `LiveExecutorSet::build` (factory.rs:66) does `crate::live_executor_set().await`, and
`live_executor_set` (host.rs) builds a FRESH `ReqwestHttpTransport::new()` (reqwest client) +
`BedrockModelTransport::new().await` (AWS config/IMDS load) EVERY call. `build()` is invoked once per
install (the `ExecutorSetBuilder` seam in `ComponentSessionFactory`), and installs run in the daemon's
single-threaded host-loop admin handler (the `!Send` registry loop). So each install synchronously awaits a
full transport rebuild + AWS default resolution — a slow/timing-out IMDS or config probe blocks the whole
loop, stalling every session's inbound processing + timers for the probe's duration. MED/availability —
scoped to `live-net` (the real credentialed daemon) and installs may be infrequent, but the single-threaded
stall is the specific hazard, and IMDS probes can be multi-second on a cold/misconfigured host. Fix per
Copilot: build the live transports + `SdkConfig`/Bedrock client ONCE at daemon startup; have
`LiveExecutorSet` hold + CLONE the shared `reqwest::Client` + Bedrock client into a fresh
`CompositeExecutor` per session (reqwest clients are cheap to clone — a shared connection pool; `SdkConfig`
clones cheaply too). That moves the one-time cost to boot and makes per-install `build()` allocation-cheap +
non-blocking. v-agent-harness-host owns cdz-agent-host/src.
