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
//! the channel, and writes response frames back — the default build binds no socket. Every command is
//! authorize-then-apply gated: the loop calls `apply_admin_authorized`, which runs the configured
//! [`AdminAuthorizer`] on the command's action + principal BEFORE mutating (a denied command never touches
//! the registry). The deployed daemon installs the trusted-local-admin [`AllowList`]; swapping in a
//! Cedar-policy-component authorizer (reusing the kernel `ComponentAuthorizer` path) is the following slice.
//!
//! All executor errors are RECOVERABLE + classified for the kernel's supervision tree:
//! an `EffectOutcome::Err` carries a typed [`Retryability`](cdz_kernel::event::Retryability) field so a
//! supervisor decides backoff-retry vs give-up — never a panic, never a silent drop (the operator's
//! error-resilience floor).
//!
//! The shared surface with `cdz-kernel` is ONLY the trait signatures; this crate never edits kernel src.

pub mod admin;
#[cfg(feature = "admin")]
pub mod admin_socket;
pub mod admin_wire;
pub mod async_host;
pub mod blob_cache;
pub mod blob_exec;
pub mod clock;
pub mod config;
pub mod converse;
#[cfg(feature = "live-net")]
pub mod converse_json;
pub mod disk_cache;
#[cfg(feature = "live-aws-storage")]
pub mod dynamo_log;
pub mod effect_registry;
pub mod effect_reply;
pub mod emit;
#[cfg(feature = "metrics-export")]
pub mod export;
pub mod factory;
#[cfg(feature = "live-fs")]
pub mod fs_exec;
pub mod host;
pub mod http;
pub mod lifecycle;
pub mod metric_exec;
pub mod metrics;
pub mod model;
pub mod name_snapshot;
pub mod reply_exec;
#[cfg(feature = "live-aws-storage")]
pub mod s3_blob;
pub mod session_registry;
#[cfg(feature = "live-exec")]
pub mod shell;
pub mod status;
#[cfg(test)]
mod test_support;
pub mod userspace_effect_exec;
#[cfg(feature = "live-ws")]
pub mod ws_dial;
pub mod ws_exec;
#[cfg(feature = "live-ws")]
pub mod ws_listen;
pub mod ws_socket;

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
pub use blob_cache::CachingBlobStore;
pub use blob_exec::BlobExecutor;
pub use clock::ClockExecutor;
pub use config::{
    AdminConfig, BlobCacheConfig, BlobConfig, DaemonConfig, LogConfig, MetricsTarget,
    NameStoreConfig, ObservabilityConfig, RetryConfig, SessionRegistryConfig, TracingConfig,
    TracingFormat, TracingOutput,
};
pub use disk_cache::DiskCacheTier;
#[cfg(feature = "live-aws-storage")]
pub use dynamo_log::{DynamoLogSink, DynamoLogSinkBuilder};
pub use effect_registry::{effect_families_owned_by, resolve_handler_session, CanonicalResolver};
pub use effect_reply::{ReplyTarget, ReplyTokenRegistry};
pub use emit::EmitExecutor;
#[cfg(feature = "metrics-export-prometheus")]
pub use export::{is_loopback_bind, run_prometheus_scrape_server, PrometheusTarget};
#[cfg(feature = "metrics-export-cloudwatch")]
pub use export::{run_cloudwatch_forwarder, CloudWatchBackend, CloudWatchDatum, CloudWatchTarget};
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
#[cfg(feature = "live-fs")]
pub use fs_exec::FsExecutor;
#[cfg(feature = "live-net")]
pub use host::live_executor_set;
pub use host::{genesis_ct, AgentHost, HostedSession, SessionId};
#[cfg(feature = "live-net")]
pub use http::ReqwestHttpTransport;
pub use http::{HttpExecutor, HttpMethod, HttpResponse, HttpTransport};
pub use lifecycle::{LifecycleChannel, LifecycleExecutor, LifecycleOp};
pub use metric_exec::MetricExecutor;
pub use metrics::{EffectMetrics, HostMetrics};
#[cfg(feature = "live-net")]
pub use model::BedrockModelTransport;
pub use model::{ModelExecutor, ModelTransport};
#[cfg(feature = "live-aws-storage")]
pub use name_snapshot::S3NameStoreSnapshot;
pub use name_snapshot::{MemNameStoreSnapshot, NameStoreSnapshotStore};
pub use reply_exec::{reply_settle_channel, ReplyExecutor, ReplySettle, ReplySettleSink};
// Retryability is the kernel's typed field on `EffectOutcome::Err` (operator Q2); re-export it directly
// from the kernel — the old crate-local `retry` module was a pure re-export shim (no logic), now deleted.
pub use cdz_kernel::event::Retryability;
#[cfg(feature = "live-aws-storage")]
pub use s3_blob::S3BlobStore;
#[cfg(feature = "live-aws-storage")]
pub use session_registry::DynamoSessionRegistry;
pub use session_registry::{SessionRecord, SessionStatus};
#[cfg(feature = "live-exec")]
pub use shell::ShellExecutor;
pub use status::{host_session_status_json, session_status_json, DEFAULT_STALL_AFTER_MS};
pub use userspace_effect_exec::{
    effect_request_inbound, HandlerResolver, UserspaceEffectExecutor, EFFECT_REQUEST_FAMILY_PREFIX,
    EFFECT_REQUEST_VERSION,
};
#[cfg(feature = "live-ws")]
pub use ws_dial::{dial_hub, WsDialExecutor};
pub use ws_exec::{WsConnRegistry, WsSendExecutor, WsSendResult};
#[cfg(feature = "live-ws")]
pub use ws_listen::WsListener;
pub use ws_socket::{
    emit_ws_event, mint_conn_id, ws_connect_inbound, ws_control_channel, ws_disconnect_inbound,
    ws_frame_inbound, LiveWsConnRegistry, OutboundFrameSink, WsControlOp, WsControlReceiver,
    WsControlSender, WS_EVENT_VERSION, WS_FRAME_FAMILY,
};

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

    /// A host-side recording [`LogSink`](cdz_kernel::log_store::LogSink) test fixture — mirrors the kernel's
    /// crate-private `test_log_source` seam (log/state-decouple I5). Once the kernel drops the resident log
    /// Vec + removes `Session::log()`, a test that needs the full durable log reads it from the SOURCE (the
    /// attached sink) rather than the resident Vec — exactly as production recovery reads it from
    /// `LogStore::recover`. The kernel fixture is `pub(crate)` to the kernel, so the host mirrors it over the
    /// public seam (`Session::attach_sink` + `Session::genesis_ref` + the `LogSink` trait).
    #[cfg(test)]
    pub(crate) mod log_capture {
        use cdz_kernel::event::Event;
        use cdz_kernel::kernel::Session;
        use std::cell::RefCell;
        use std::rc::Rc;

        /// The captured event buffer, shared between the attached sink and the test that reads it. `Rc` +
        /// `RefCell` (not `Arc`) — the kernel is single-threaded by design (the `?Send` backends).
        pub(crate) type CapturedLog = Rc<RefCell<Vec<Event>>>;

        struct MemLogSink {
            captured: CapturedLog,
        }

        #[async_trait::async_trait(?Send)]
        impl cdz_kernel::log_store::LogSink for MemLogSink {
            async fn append(&mut self, event: &Event) -> std::io::Result<()> {
                self.captured.borrow_mut().push(event.clone());
                Ok(())
            }
        }

        /// Attach a fresh recording sink to `session`, seeded with its genesis, and return the shared
        /// buffer. After this every appended event is captured; the test replays from [`replay_input`] to
        /// prove the durable-log SOURCE (not the resident Vec) reconstructs the session. Call on a FRESH
        /// `genesis` session, before delivering events.
        pub(crate) fn attach_recording_sink(session: &mut Session) -> CapturedLog {
            let captured: CapturedLog = Rc::new(RefCell::new(vec![session.genesis_ref().clone()]));
            session.attach_sink(Box::new(MemLogSink {
                captured: Rc::clone(&captured),
            }));
            captured
        }

        /// The captured durable log as an owned `Vec<Event>` — what a test hands to
        /// [`Session::replay`](cdz_kernel::kernel::Session::replay) in place of `session.log().to_vec()`.
        /// Reading from the SOURCE, not the resident Vec.
        pub(crate) fn replay_input(captured: &CapturedLog) -> Vec<Event> {
            captured.borrow().clone()
        }

        /// A bare recording sink + its shared buffer, for the `HostedSession::with_sink` builder path (which
        /// attaches POST-genesis, exactly like a production durable sink — so the genesis event predates the
        /// sink and isn't captured; the buffer holds the events appended during subsequent turns). Use this
        /// when a test needs to inspect the durable log of a `HostedSession` (which owns its `Session`) rather
        /// than a bare `Session`. Returns `(sink_to_attach, captured_buffer)`.
        pub(crate) fn recording_sink() -> (Box<dyn cdz_kernel::log_store::LogSink>, CapturedLog) {
            let captured: CapturedLog = Rc::new(RefCell::new(Vec::new()));
            (
                Box::new(MemLogSink {
                    captured: Rc::clone(&captured),
                }),
                captured,
            )
        }

        /// Like [`recording_sink`] but PRE-SEEDED with `genesis`, so the captured buffer is the COMPLETE
        /// durable log (genesis + subsequent appends) — a full [`Session::replay`] input. The
        /// `HostedSession::with_sink` builder attaches post-genesis, so a production durable sink misses the
        /// genesis event; a recovery-EQUIVALENCE test needs the whole log including genesis (replay requires a
        /// `Genesis` at `events[0]`), so it seeds it here from the session's [`genesis_ref`]. Returns
        /// `(sink_to_attach, captured_buffer)`.
        pub(crate) fn recording_sink_seeded(
            genesis: Event,
        ) -> (Box<dyn cdz_kernel::log_store::LogSink>, CapturedLog) {
            let captured: CapturedLog = Rc::new(RefCell::new(vec![genesis]));
            (
                Box::new(MemLogSink {
                    captured: Rc::clone(&captured),
                }),
                captured,
            )
        }
    }
}
