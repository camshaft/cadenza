//! cdz-agent-host — the REAL effect executors + authorizer that let a `cdz-kernel` session run against
//! the world.
//!
//! The kernel (`cdz-kernel`) is a generic if-this-then-that spine: it folds a reducer over events,
//! authorizes each requested effect, durably dispatches it, and folds the result back. It defines the
//! [`cdz_kernel::executor::Executor`] and [`cdz_kernel::authz::Authorize`] TRAITS but ships only
//! test/interim impls (a `RecordingExecutor`, the `live-exec` `ShellExecutor`, a flat capability
//! `Authorizer`). The ONE thing missing to make an agent LOOP end-to-end is a set of REAL executors:
//! when a reducer's `Model` effect actually calls Bedrock and folds the completion back, an agent runs.
//!
//! This crate provides those executors, layered so the default build stays hermetic (no network, no
//! credentials — the same discipline `cdz-kernel`'s `live-exec` feature uses):
//! - [`ClockExecutor`] — `Now` → the system wall clock. Hermetic (no feature gate): it needs no network
//!   and no credentials, and its result is RECORDED in the log, so replay reuses the frozen instant
//!   (§9c reads-are-effects — the reducer never reads the clock directly; determinism lives in the log,
//!   not the executor).
//! - [`ModelExecutor`] — `Model` → a model completion, the headline: when a reducer's `Model` effect
//!   reaches a model and the completion folds back, an agent loops end-to-end. It is GENERIC over a
//!   [`ModelTransport`] so its effect-mapping logic is hermetically testable; the real Bedrock transport
//!   (SigV4 + the cred-broker) lands behind `live-net`, a stub drives the default gate.
//! - [`HttpExecutor`] — `Http` → an HTTP response, for an agent that fetches a URL. Same transport-seam
//!   shape as the model executor (generic over an [`HttpTransport`]); the real client lands behind
//!   `live-net`, a stub drives the default gate. The URL's host is gated by the kernel's SEC-F1 `HostIn`
//!   capability (the SSRF/exfil guard) before dispatch, so this executor does not re-authorize.
//!
//! And [`AgentHost`] (see [`host`]) is what ASSEMBLES those pieces into a process that actually RUNS
//! agents: a registry of live kernel `Session`s keyed by id, each driven by its reducer + a
//! `CompositeExecutor` of the real executors + an authorizer. Delivering an inbound event runs one turn
//! of the reactive loop (deliver → fold → authorize → dispatch → fold-result) — an agent, running.
//! [`AsyncAgentHost`] (see [`async_host`]) wraps that registry in a SINGLE-THREADED async event loop that
//! interleaves MANY sessions on one task (a `select!` over a shared inbox + a cross-session timer wheel +
//! shutdown) — the multi-session daemon, built so v-agent-harness's kernel-async conversion swaps the
//! in-place `deliver` for `.await` with no reshape (single-threaded, no `Send`).
//!
//! The deployed daemon boots as a pure CONTROL PLANE (empty registry) and an ADMIN installs + manages
//! sessions at runtime over an admin control interface (operator directive). [`admin`] is the transport-
//! agnostic command layer of that: an [`AdminCommand`] (install-session / list / status / stop) applied
//! by [`AgentHost::apply_admin`](host::AgentHost::apply_admin) via a [`SessionFactory`] install seam.
//! [`admin_wire`] is the codec that carries those commands/responses over a control frame — serde-native
//! wire DTOs (decoupled from the domain types, ids as strings + hashes as hex) + a length-prefixed JSON
//! frame codec. The [`AsyncAgentHost`] loop drains admin commands from an in-process channel
//! ([`AdminChannel`]) alongside its inbox + timers, applying each on the loop
//! task (where the `!Send` registry lives) and replying via a per-command oneshot — so a `Send`
//! socket-listener task can drive the control interface without the registry ever crossing a thread. The
//! `admin_socket` module (behind the `admin` cargo feature) is that transport: an `AdminSocket` binds a
//! local Unix-domain socket (owner-only perms), reads length-prefixed command frames, forwards them over
//! the channel, and writes response frames back — the default build binds no socket. The Cedar `admin/*`
//! authorization of each command is the following slice.
//!
//! All executor errors are RECOVERABLE + classified for the kernel's supervision tree (see [`retry`]):
//! an `EffectOutcome::Err` reason leads with a `RETRYABLE:`/`PERMANENT:` token so a supervisor decides
//! backoff-retry vs give-up — never a panic, never a silent drop (the operator's error-resilience floor).
//!
//! The shared surface with `cdz-kernel` is ONLY the trait signatures; this crate never edits kernel src.

pub mod admin;
#[cfg(feature = "admin")]
pub mod admin_socket;
pub mod admin_wire;
pub mod async_host;
pub mod clock;
pub mod config;
pub mod emit;
#[cfg(feature = "metrics-export")]
pub mod export;
pub mod factory;
pub mod host;
pub mod http;
pub mod lifecycle;
pub mod metrics;
pub mod model;
pub mod retry;
pub mod status;

pub use admin::{
    AdminAuthorizer, AdminCommand, AdminResponse, AllowList, InstallSpec, SessionFactory,
};
#[cfg(feature = "admin")]
pub use admin_socket::{AdminSocket, LOCAL_ADMIN_PRINCIPAL};
pub use admin_wire::{
    decode_frame, encode_frame, AdminCommandWire, AdminResponseWire, InstallSpecWire, WireError,
    MAX_FRAME_LEN,
};
pub use async_host::{AdminChannel, AdminRequest, AsyncAgentHost, Inbound, Inbox};
pub use clock::ClockExecutor;
pub use config::{
    AdminConfig, BlobConfig, DaemonConfig, LogConfig, MetricsTarget, ObservabilityConfig,
    RetryConfig, TracingConfig, TracingFormat, TracingOutput,
};
pub use emit::EmitExecutor;
#[cfg(feature = "metrics-export-prometheus")]
pub use export::{is_loopback_bind, run_prometheus_scrape_server, PrometheusTarget};
#[cfg(feature = "metrics-export")]
pub use export::{run_export_loop, MetricsReporter, StatsdTarget, UdpStatsdSink};
#[cfg(feature = "metrics-export-otlp")]
pub use export::{run_otlp_forwarder, ChannelOtlpSink, OtlpTarget};
#[cfg(feature = "live-net")]
pub use factory::LiveExecutorSet;
pub use factory::{
    AuthorizerBuilder, ComponentSessionFactory, ExecutorSetBuilder, FileLogSinkBuilder,
    LogSinkBuilder, MeteredExecutor,
};
#[cfg(feature = "live-net")]
pub use host::live_executor_set;
pub use host::{genesis_ct, AgentHost, HostedSession, SessionId};
#[cfg(feature = "live-net")]
pub use http::ReqwestHttpTransport;
pub use http::{HttpExecutor, HttpMethod, HttpResponse, HttpTransport};
pub use lifecycle::{LifecycleChannel, LifecycleExecutor, LifecycleOp};
pub use metrics::{EffectMetrics, HostMetrics};
#[cfg(feature = "live-net")]
pub use model::BedrockModelTransport;
pub use model::{ModelExecutor, ModelTransport};
pub use retry::{classify, permanent, retryable, Retryability};
pub use status::{host_session_status_json, session_status_json, DEFAULT_STALL_AFTER_MS};

/// Shared test-only helpers (compiled only under `#[cfg(test)]`; placed LAST so no non-test item follows a
/// test module — clippy `items_after_test_module`). The one PROVEN-FRESH temp-dir helper the crate's
/// fs-touching tests use, so no test writes to a fixed path (parallel-collision / crashed-run leftover
/// flakiness — the #1988/#1991/#1995 review family).
#[cfg(test)]
pub(crate) mod testutil {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A UNIQUE, PROVEN-FRESH temp dir for a test run: `<tmp>/cdz-<tag>-<pid>-<seq>`, created with
    /// `create_dir` (NOT `create_dir_all`) so it must NOT already exist — on `AlreadyExists` (a crashed
    /// prior run whose pid the OS reused + a seq collision) it bumps the seq and retries, so the returned
    /// dir is guaranteed fresh (never a silently-reused stale dir). The caller `remove_dir_all`s it when
    /// done. No `Date`/`rand` needed — pid + a process-local atomic counter is unique per run + call.
    pub(crate) fn unique_temp_dir(tag: &str) -> PathBuf {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        loop {
            let dir = std::env::temp_dir().join(format!(
                "cdz-{tag}-{}-{}",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&dir) {
                Ok(()) => return dir,
                // The dir already exists (pid reuse + seq collision from a crashed run) — bump seq + retry
                // so we PROVE a fresh dir rather than reuse a stale one.
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("could not create a unique temp dir for {tag}: {e}"),
            }
        }
    }
}
