//! The real [`SessionFactory`] — turns an admin `install-session` command into a running agent by LOADING
//! its reducer from the blob store.
//!
//! Building a reducer from a `reducer_hash` means loading its wasm component, which is a HOST concern — the
//! kernel exposes the pieces ([`BlobStore::get`] + [`AsyncComponentReducer::from_component_bytes`]) and the
//! host assembles them behind the [`SessionFactory`] seam. This is that assembly:
//! [`ComponentSessionFactory`] holds a [`BlobStore`] + the per-session executor set, and on each install it
//!
//! 1. fetches the reducer component bytes by content hash (`blob.get(reducer_hash)`),
//! 2. lifts them into an [`AsyncComponentReducer`] (`from_component_bytes` — the kernel's reducer-from-bytes
//!    seam), and
//! 3. assembles a [`HostedSession`] via [`HostedSession::genesis`] with that reducer + a fresh copy of the
//!    executor set + the session's authorizer.
//!
//! It is GENERIC over the blob store (so a test drives a [`MemBlobStore`](cdz_kernel::blob::MemBlobStore)
//! with a real component + the deployed daemon a durable backend) and takes the executor set + authorizer
//! as caller-supplied builders, so the factory itself stays hermetically testable — the network/AWS
//! executor tree lives in the daemon's `live_executor_set()` (behind `live-net`), NOT here. A malformed or
//! absent component is a clean `Err(String)` (surfaced as an [`AdminResponse::Error`](crate::AdminResponse)),
//! never a panic.

use crate::admin::{InstallSpec, SessionFactory};
use crate::host::HostedSession;
use crate::metrics::EffectMetrics;
use cdz_kernel::authz::Authorize;
use cdz_kernel::blob::BlobStore;
use cdz_kernel::effect::EffectRequest;
use cdz_kernel::event::EffectOutcome;
use cdz_kernel::executor::{CompositeExecutor, Executor};
use cdz_kernel::hash::Hash;
use cdz_kernel::wasm_host::AsyncComponentReducer;
use std::sync::Arc;

/// Builds a per-session executor set (the effects a freshly-installed session may perform). Called ONCE per
/// install so each session gets its OWN [`CompositeExecutor`] — the kernel's `Executor::perform` takes
/// `&mut self` and [`HostedSession`] owns its executor by value, so sessions can't share one set. The
/// deployed daemon returns the live set (Clock + HTTP + Bedrock), a test returns a hermetic one.
///
/// ASYNC (`?Send`, matching the crate's single-threaded convention): the deployed daemon's builder is
/// `LiveExecutorSet` (behind `live-net`), whose `build` `.await`s the live executor assembly (it loads AWS
/// config for the Bedrock transport). A synchronous builder (a plain closure) satisfies it with no real
/// await via the blanket impl.
/// `build` takes the installed session's `id` (the registry label) AND its `owner_genesis` (the session's
/// genesis hash) so a per-session executor that needs the session identity — the
/// [`LifecycleExecutor`](crate::LifecycleExecutor), whose `by` is the id + whose spawn-provenance is the
/// genesis hash — is built correctly. Both are threaded because the id is an opaque label (may be a vanity
/// string) while the genesis hash is the stable provenance a spawn derives from (never re-parse the id as
/// hex; §tick-#784). The lifecycle CHANNEL (the loop's) is held by the concrete builder (e.g.
/// [`LiveExecutorSet`], set at construction), one-per-loop. A builder serving no lifecycle effects ignores both.
#[async_trait::async_trait(?Send)]
pub trait ExecutorSetBuilder {
    async fn build(&self, id: &crate::host::SessionId, owner_genesis: Hash) -> CompositeExecutor;
}

/// A synchronous `Fn() -> CompositeExecutor` (a test's hermetic set, or any non-async assembly) is an
/// `ExecutorSetBuilder` for free — its `build` just calls the closure (no real await). It ignores `id` +
/// `owner_genesis` (a closure-built set serves no per-session-identity executor; a caller that needs one
/// uses a real builder).
#[async_trait::async_trait(?Send)]
impl<F: Fn() -> CompositeExecutor> ExecutorSetBuilder for F {
    async fn build(&self, _id: &crate::host::SessionId, _owner_genesis: Hash) -> CompositeExecutor {
        self()
    }
}

/// A metering DECORATOR over an [`Executor`] — the executor-level seam for the PER-EFFECT half of the
/// metric surface (see [`crate::metrics::EffectMetrics`]). It wraps a real executor and, on every
/// `perform`, dispatches to the inner one and then tallies the outcome into a shared [`EffectMetrics`]:
/// `Ok` → `record_ok`, `Err(reason)` → `record_err(classify(reason))` (the same [`crate::retry`]
/// classification a supervisor branches on). Transparent otherwise — the outcome is returned unchanged and
/// `handles_family` forwards, so wrapping is invisible to the kernel.
///
/// This is why the per-effect counts need no kernel change: every dispatch already flows through
/// `Executor::perform`, so metering at the executor boundary (where the outcome is produced) captures it.
/// The deployed daemon's `LiveExecutorSet::build` wraps each real leaf executor (Clock/Http/Model) with one
/// of these, all sharing ONE host-wide `Arc<EffectMetrics>` (registered from the daemon's metrics registry),
/// so every session's ok/retryable/permanent/timed-out tallies aggregate across all effect families into a
/// daemon-wide set the export backend reports over. A non-daemon caller (a test, an embedder) OPTS IN the
/// same way — wrap an executor + share an `Arc`.
pub struct MeteredExecutor {
    inner: Box<dyn Executor>,
    metrics: Arc<EffectMetrics>,
}

impl MeteredExecutor {
    /// Wrap `inner`, tallying each outcome into `metrics`. Boxes ready to register with
    /// [`CompositeExecutor::with_effect`].
    pub fn new(inner: Box<dyn Executor>, metrics: Arc<EffectMetrics>) -> Self {
        MeteredExecutor { inner, metrics }
    }
}

#[async_trait::async_trait(?Send)]
impl Executor for MeteredExecutor {
    async fn perform(&mut self, req: &EffectRequest, idempotency_key: Hash) -> EffectOutcome {
        let started = std::time::Instant::now();
        let outcome = self.inner.perform(req, idempotency_key).await;
        self.metrics
            .record_latency_us(crate::metrics::micros_u64(started.elapsed()));
        // Tally the metric + trace at the SAME tap. The family names which effect; a failure logs its
        // classified reason (retryable/permanent) at warn (a supervisor signal), ok at debug (routine).
        let family = req.content_type.family.as_ref();
        match &outcome {
            EffectOutcome::Ok(_) => {
                self.metrics.record_ok();
                tracing::debug!(target: "cdz_agent_host::effect", family, "effect ok");
            }
            EffectOutcome::Err {
                message,
                retryability,
            } => {
                // Read the TYPED retryability straight off the outcome (no string parsing) for both the
                // metric and the trace, so the event carries it as a structured field — #2069 review.
                self.metrics.record_err(*retryability);
                tracing::warn!(
                    target: "cdz_agent_host::effect",
                    family,
                    retryability = ?retryability,
                    reason = message,
                    "effect errored"
                );
            }
            EffectOutcome::TimedOut => {
                self.metrics.record_timed_out();
                tracing::warn!(target: "cdz_agent_host::effect", family, "effect timed out");
            }
        }
        outcome
    }

    /// Forward the mechanism probe so wrapping doesn't hide the inner executor's served family from the
    /// capability manifest (a wrapped executor must still report what it serves).
    fn handles_family(&self, family: &str) -> bool {
        self.inner.handles_family(family)
    }
}

/// The deployed daemon's [`ExecutorSetBuilder`]: assemble the LIVE executor set (Clock + HTTP + Bedrock,
/// env creds) per install by CLONING transports built ONCE at daemon startup. Behind `live-net`.
///
/// **Build-once, clone-per-install (#1987 review).** A prior version called `live_executor_set().await` in
/// every `build()` — which rebuilt the reqwest client + re-ran the AWS/IMDS config load EACH install, inside
/// the single-threaded host loop; a slow/timing-out IMDS probe then stalled every session. Now the daemon
/// constructs the HTTP + Bedrock transports ONCE ([`LiveExecutorSet::new`], which `.await`s the AWS load at
/// boot) and each `build()` just CLONES them (a reqwest `Client` / aws-sdk client clone is an `Arc` refcount
/// bump over a shared pool — cheap + non-blocking) into a fresh `CompositeExecutor`. The per-install cost is
/// now a couple of cheap clones, not a network probe.
#[cfg(feature = "live-net")]
pub struct LiveExecutorSet {
    http: crate::ReqwestHttpTransport,
    model: crate::BedrockModelTransport,
    /// The HOST-WIDE per-effect metric tally (matching [`HostMetrics`](crate::HostMetrics)' host-wide
    /// scope). Registered from the daemon's shared registry at boot; every per-install executor is wrapped in
    /// a [`MeteredExecutor`] sharing a CLONE of this `Arc`, so all sessions' effect outcomes aggregate into
    /// one daemon-wide set the exporter reports over.
    metrics: Arc<EffectMetrics>,
    /// A clone of the daemon's shared metrics [`Registry`] (the SAME one `metrics` registered into + the
    /// exporter drains) — held so `build` can construct a [`MetricExecutor`](crate::MetricExecutor) per
    /// session for the `metric/publish` effect, letting a reducer publish metrics that reach the configured
    /// backends. Cheap to clone (an `Arc` into the registry storage).
    registry: crate::metrics::Registry,
    /// The loop's lifecycle channel, if the daemon wired one (§lifecycle). Set via
    /// [`with_lifecycle_channel`](Self::with_lifecycle_channel) at boot from
    /// [`AsyncAgentHost::lifecycle_channel`](crate::AsyncAgentHost::lifecycle_channel). When present, `build`
    /// wires the `lifecycle/*` executors (spawn/suspend/resume/terminate) for each installed session, owned
    /// by that session's id — so a deployed session can perform lifecycle effects. `None` (the default / a
    /// pure control-plane build with no loop channel) = no lifecycle executor is wired.
    lifecycle: Option<crate::lifecycle::LifecycleChannel>,
}

#[cfg(feature = "live-net")]
impl LiveExecutorSet {
    /// Build the shared transports ONCE (at daemon startup) — this is where the AWS config / IMDS load
    /// happens, on the boot path, NOT per install. `Err` if the HTTP client can't be built (e.g. no TLS
    /// backend) — a fatal daemon misconfiguration surfaced at boot, not per session. The per-effect
    /// [`EffectMetrics`] register into `registry` (the daemon's — the SAME one
    /// [`AgentHost`](crate::AgentHost) records host metrics into), so all metrics share one registry the
    /// exporter reports over. No lifecycle channel by default — add one with
    /// [`with_lifecycle_channel`](Self::with_lifecycle_channel) to serve `lifecycle/*` effects.
    pub async fn new(registry: &crate::metrics::Registry) -> Result<Self, String> {
        let http = crate::ReqwestHttpTransport::new()?;
        let model = crate::BedrockModelTransport::new().await;
        Ok(LiveExecutorSet {
            http,
            model,
            metrics: Arc::new(EffectMetrics::new(registry)),
            registry: registry.clone(),
            lifecycle: None,
        })
    }

    /// Wire the loop's lifecycle channel so each installed session gets the `lifecycle/*` executors
    /// (spawn/suspend/resume/terminate). Called ONCE at daemon boot with
    /// [`AsyncAgentHost::lifecycle_channel`](crate::AsyncAgentHost::lifecycle_channel). Without this, a built
    /// session serves no lifecycle effects (a pure control-plane / test build).
    pub fn with_lifecycle_channel(mut self, lifecycle: crate::lifecycle::LifecycleChannel) -> Self {
        self.lifecycle = Some(lifecycle);
        self
    }

    /// The host-wide per-effect metrics this set's executors tally into — shared, so the returned handle
    /// observes every session's dispatches. (Held so the wrap in `build` shares one `Arc`.)
    pub fn metrics(&self) -> Arc<EffectMetrics> {
        self.metrics.clone()
    }
}

#[cfg(feature = "live-net")]
#[async_trait::async_trait(?Send)]
impl ExecutorSetBuilder for LiveExecutorSet {
    async fn build(&self, id: &crate::host::SessionId, owner_genesis: Hash) -> CompositeExecutor {
        use cdz_kernel::effect::effect_ct;
        // Cheap per-install assembly: CLONE the shared transports (Arc bumps) into fresh executors, each
        // WRAPPED in a MeteredExecutor sharing the host-wide Arc<EffectMetrics> (an Arc-clone, also cheap) so
        // every session's ok/retryable/permanent/timed-out outcomes tally into one daemon-wide set. No
        // network / IMDS work here, so it never stalls the loop.
        let m = &self.metrics;
        let composite = CompositeExecutor::new()
            .with_effect(
                effect_ct::NOW,
                Box::new(MeteredExecutor::new(
                    Box::new(crate::ClockExecutor::new()),
                    m.clone(),
                )),
            )
            .with_effect(
                effect_ct::HTTP,
                Box::new(MeteredExecutor::new(
                    Box::new(crate::HttpExecutor::new(self.http.clone())),
                    m.clone(),
                )),
            )
            .with_effect(
                effect_ct::MODEL,
                Box::new(MeteredExecutor::new(
                    Box::new(crate::ModelExecutor::new(self.model.clone())),
                    m.clone(),
                )),
            )
            // Q3: wire the `metric/publish` executor UNCONDITIONALLY — it lets a reducer publish a metric that
            // the host records into the shared Registry (so it drains to the configured backends alongside the
            // host's own metrics). Same posture as shell/fs: WHICH metrics a session may publish is EVOLVABLE
            // Cedar policy on the effect's resolved metric-name target (operator standing-order), so wiring the
            // mechanism for all is safe — a session records only what its policy permitted. The executor holds
            // its own per-session cardinality + guest-string-safety bound (the host's mechanism). Metered like
            // the others. Each session gets a fresh MetricExecutor (its own cardinality set) over a clone of
            // the shared Registry.
            .with_effect(
                effect_ct::METRIC_PUBLISH,
                Box::new(MeteredExecutor::new(
                    Box::new(crate::MetricExecutor::new(self.registry.clone())),
                    m.clone(),
                )),
            );
        // GAP-2: wire the `shell` executor UNCONDITIONALLY (behind `live-exec`). No host-side allow-list or
        // config gate — WHICH commands a session may run is EVOLVABLE POLICY the Cedar wasm authorizer decides
        // on the effect's resolved target BEFORE dispatch (operator standing-order: host = inevolvable
        // MECHANISM, policy = wasm on the log). A session with no `shell` capability in its policy simply
        // never has a shell effect authorized, so wiring the mechanism for all is safe: the executor runs only
        // what Cedar already permitted. Metered like the others.
        #[cfg(feature = "live-exec")]
        let composite = composite.with_effect(
            effect_ct::SHELL,
            Box::new(MeteredExecutor::new(
                Box::new(crate::ShellExecutor::new()),
                m.clone(),
            )),
        );
        // GAP-3: wire the `fs/*` executor UNCONDITIONALLY (behind `live-fs`). Same posture as shell: WHICH
        // paths a session may read/write is EVOLVABLE Cedar policy on the effect's resolved target (operator
        // standing-order), so wiring the mechanism for all is safe — a session runs only the fs ops its
        // policy permitted. The `CompositeExecutor` routes by EXACT family (not prefix), so register the
        // FsExecutor under each fs verb (read/write/glob); it's a unit struct, so a fresh one per verb is
        // free. Metered like the others.
        #[cfg(feature = "live-fs")]
        let composite = {
            let mut c = composite;
            for fam in [effect_ct::FS_READ, effect_ct::FS_WRITE, effect_ct::FS_GLOB] {
                c = c.with_effect(
                    fam,
                    Box::new(MeteredExecutor::new(
                        Box::new(crate::FsExecutor::new()),
                        m.clone(),
                    )),
                );
            }
            c
        };
        // §lifecycle (tick-#784 gap): when the daemon wired the loop's lifecycle channel, register the four
        // `lifecycle/*` executors so an installed session can spawn/suspend/resume/terminate. Each is owned
        // by THIS session's id (the `by` stamped on its lifecycle ops); the LifecycleExecutor records a
        // LifecycleOp on the channel and the loop applies it (defer-to-loop). The CompositeExecutor routes by
        // EXACT family, so one executor per verb (each holds a clone of the channel + the owner id). NO host
        // policy — whether a session MAY lifecycle a target is the Cedar authority (I6 DescendantOf), gated
        // before dispatch. `None` channel (control-plane/test build) = no lifecycle effect served.
        if let Some(lifecycle) = &self.lifecycle {
            let mut c = composite;
            for fam in [
                effect_ct::LIFECYCLE_SPAWN,
                effect_ct::LIFECYCLE_SUSPEND,
                effect_ct::LIFECYCLE_RESUME,
                effect_ct::LIFECYCLE_TERMINATE,
            ] {
                c = c.with_effect(
                    fam,
                    Box::new(MeteredExecutor::new(
                        Box::new(crate::LifecycleExecutor::new(
                            lifecycle.clone(),
                            id.clone(),
                            owner_genesis,
                        )),
                        m.clone(),
                    )),
                );
            }
            c
        } else {
            composite
        }
    }
}

/// Builds a per-session authorizer (the policy gating what an installed session may do). Called ONCE per
/// install. The deployed daemon derives it from the configured policy; a test returns a fixed one. Kept
/// separate from the executor set because "what a session CAN do" (mechanism) and "what it MAY do" (policy)
/// are the two independent axes the kernel authorizes on.
pub trait AuthorizerBuilder {
    fn build(&self) -> Box<dyn Authorize>;
}

impl<F: Fn() -> Box<dyn Authorize>> AuthorizerBuilder for F {
    fn build(&self) -> Box<dyn Authorize> {
        self()
    }
}

/// Builds a per-session DURABLE-LOG sink — where an installed session's event log persists. Called ONCE per
/// install, keyed by the session id (so a file backend derives a per-session path, e.g.
/// `<log-dir>/<id>.log`). Returns `None` for a session that should keep its in-memory log only (the `memory`
/// log backend, dev/test), or `Err` if opening the durable log failed (surfaced as an install error — an
/// un-openable log is a real misconfig). Async (`?Send`) to match the crate convention (a network log
/// backend `.await`s; a disk `LogStore::open` returns ready).
#[async_trait::async_trait(?Send)]
pub trait LogSinkBuilder {
    async fn build(
        &self,
        id: &crate::host::SessionId,
    ) -> Result<Option<Box<dyn cdz_kernel::log_store::LogSink>>, String>;
}

/// A [`LogSinkBuilder`] that opens a per-session file-backed [`LogStore`](cdz_kernel::log_store::LogStore)
/// under a root directory — the `[log].backend = file` durable log. Each session's log is
/// `<root>/<session-id>.log`; the session id is sanitized (path separators → `_`) so an id can't escape the
/// root. The deployed daemon installs this when the log config selects a file backend.
pub struct FileLogSinkBuilder {
    root: std::path::PathBuf,
}

impl FileLogSinkBuilder {
    /// Root the per-session logs at `root` (created on first open by `LogStore::open`).
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        FileLogSinkBuilder { root: root.into() }
    }
}

#[async_trait::async_trait(?Send)]
impl LogSinkBuilder for FileLogSinkBuilder {
    async fn build(
        &self,
        id: &crate::host::SessionId,
    ) -> Result<Option<Box<dyn cdz_kernel::log_store::LogSink>>, String> {
        // Sanitize the id into a single safe filename component — no '/' or '\\' so a crafted id can't
        // write outside the root; a `.log` suffix keeps the files recognizable.
        let safe: String = id
            .as_str()
            .chars()
            .map(|c| if c == '/' || c == '\\' { '_' } else { c })
            .collect();
        // Ensure the root dir exists (LogStore::open opens a file, it does NOT create parent dirs — unlike
        // DiskBlobStore::open). Create it once here so a fresh [log].dir just works.
        std::fs::create_dir_all(&self.root)
            .map_err(|e| format!("could not create log dir {}: {e}", self.root.display()))?;
        let path = self.root.join(format!("{safe}.log"));
        let store = cdz_kernel::log_store::LogStore::open(&path)
            .map_err(|e| format!("could not open durable log {}: {e}", path.display()))?;
        Ok(Some(Box::new(store)))
    }
}

/// The real [`SessionFactory`]: load a reducer component from a [`BlobStore`] by hash and assemble a live
/// [`HostedSession`]. Generic over the blob store; the executor set + authorizer come from caller-supplied
/// builders (so the network/AWS tree stays in the daemon, not this hermetically-testable factory). An
/// optional [`LogSinkBuilder`] attaches a per-session durable log when configured (`[log].backend = file`);
/// without one, installed sessions keep their in-memory log (the `memory` backend / tests).
pub struct ComponentSessionFactory<B, E, A> {
    blob: B,
    executors: E,
    authz: A,
    log_sink: Option<Box<dyn LogSinkBuilder>>,
}

impl<B, E, A> ComponentSessionFactory<B, E, A>
where
    B: BlobStore,
    E: ExecutorSetBuilder,
    A: AuthorizerBuilder,
{
    /// Assemble the factory over a blob store + the per-session executor-set / authorizer builders. No
    /// durable-log sink by default (installed sessions keep their in-memory log); add one with
    /// [`with_log_sink`](Self::with_log_sink).
    pub fn new(blob: B, executors: E, authz: A) -> Self {
        ComponentSessionFactory {
            blob,
            executors,
            authz,
            log_sink: None,
        }
    }

    /// Attach a per-session durable-log [`LogSinkBuilder`] — the deployed daemon supplies one when
    /// `[log].backend = file` so each installed session's event log persists to disk. Builder-style.
    pub fn with_log_sink(mut self, log_sink: Box<dyn LogSinkBuilder>) -> Self {
        self.log_sink = Some(log_sink);
        self
    }

    /// Complete the genesis ceremony's AUTHORIZER step: resolve the authorizer-component hash the genesis
    /// reducer recorded at [`KV_AUTHORIZER_HASH`](crate::genesis_ct::KV_AUTHORIZER_HASH) and LIVE-INSTALL the
    /// real authorizer on `session`. This is the host side of §20b install-authorizer-by-hash: after
    /// [`HostedSession::seed_genesis`](crate::HostedSession::seed_genesis) folds `genesis/authorizer` into KV,
    /// the host (which holds the blob store — [`HostedSession`] stays blob-store-free) calls this to turn the
    /// recorded POINTER into an installed policy.
    ///
    /// Steps: read the raw 32-byte hash from the session's KV (the reducer stored the event payload verbatim,
    /// so the bytes are exactly what the host put in the `genesis/authorizer` event) → [`BlobStore::get`] the
    /// policy component by that hash → [`reload_policy_from_component_bytes`](crate::HostedSession::reload_policy_from_component_bytes)
    /// to lift + install it (which also pushes a `capabilities-changed` to the now-authorized agent).
    ///
    /// Returns `Ok(true)` when an authorizer was installed, `Ok(false)` when the session recorded NO
    /// authorizer-hash (a root-only genesis — nothing to install, the deny-all v0 authorizer stays). `Err` on a
    /// malformed hash (not 32 bytes), a blob-store miss/error (the recorded component isn't in the store), or a
    /// policy that doesn't lift — each a clean error the caller surfaces, never a panic. `principal` is the
    /// agent's authz principal, the same value the session's authorizer is built with.
    pub async fn install_genesis_authorizer(
        &self,
        session: &mut HostedSession,
        principal: impl Into<String>,
    ) -> Result<bool, String> {
        // Read the recorded hash out of the session KV. Copy the 32 bytes to the STACK directly (no
        // intermediate Vec heap-alloc) — try_into ends the KV borrow before the &mut-session reload below. A
        // wrong length is a clean error (the raw-32 contract enforced at resolve).
        let hash = match session
            .session()
            .kv()
            .get(crate::genesis_ct::KV_AUTHORIZER_HASH)
        {
            Some(b) => {
                let len = b.len();
                let arr: [u8; 32] = b.try_into().map_err(|_| {
                    format!(
                        "genesis authorizer-hash is {len} bytes, expected a raw 32-byte content hash"
                    )
                })?;
                cdz_kernel::hash::Hash::from_bytes(arr)
            }
            None => return Ok(false), // root-only genesis: no authorizer to install
        };
        let bytes = self
            .blob
            .get(&hash)
            .await
            .map_err(|e| format!("blob store error fetching genesis authorizer {hash}: {e}"))?
            .ok_or_else(|| {
                format!("no authorizer policy component in the blob store for genesis hash {hash}")
            })?;
        session
            .reload_policy_from_component_bytes(&bytes, principal)
            .await?;
        Ok(true)
    }

    /// Run the FULL genesis ceremony on a freshly-`genesis`'d session in ONE call: deliver the genesis-setup
    /// events ([`seed_genesis`](crate::HostedSession::seed_genesis)) then resolve + install the recorded
    /// authorizer ([`install_genesis_authorizer`](Self::install_genesis_authorizer)). This is the single
    /// host-side entry point for booting a session's authz from its bootstrap inputs — the caller supplies the
    /// established trust-root identity and (optionally) the authorizer-component hash + context, and this
    /// seeds them into the reducer's KV and turns the recorded authorizer pointer into an installed policy.
    ///
    /// Returns `Ok(true)` if an authorizer was installed (an `authorizer_hash` was provided + resolved),
    /// `Ok(false)` for a root-only boot (no authorizer recorded — the session keeps its deny-all v0 authorizer).
    /// `Err` propagates the first failure (a genesis fold error, or a malformed/absent/non-lifting authorizer).
    /// NOT atomic: the ceremony's writes are NOT rolled back on ANY later error. The setup events are delivered
    /// sequentially (root → authorizer → context) and each folds into KV as it lands, then the authorizer
    /// installs — so a failure at ANY step (a later SEED event's fold, OR the install) leaves the prior writes
    /// in place. On `Err` the session may be PARTIALLY seeded (some/all `bootstrap/*` KV present, authorizer
    /// possibly not installed). Callers should DISCARD the session on `Err` rather than reuse a half-booted one.
    /// `principal` is the agent's authz principal.
    ///
    /// Equivalent to `session.seed_genesis(root, authz_hash, ctx).await?;
    /// self.install_genesis_authorizer(session, principal).await` — a convenience so a caller drives the whole
    /// ceremony without threading the two steps (+ the KV-key contract) itself.
    pub async fn run_genesis_ceremony(
        &self,
        session: &mut HostedSession,
        root_identity: &[u8],
        authorizer_hash: Option<&[u8]>,
        context: Option<&[u8]>,
        principal: impl Into<String>,
    ) -> Result<bool, String> {
        session
            .seed_genesis(root_identity, authorizer_hash, context)
            .await
            .map_err(|e| format!("genesis seed failed: {e:?}"))?;
        self.install_genesis_authorizer(session, principal).await
    }
}

#[async_trait::async_trait(?Send)]
impl<B, E, A> SessionFactory for ComponentSessionFactory<B, E, A>
where
    B: BlobStore,
    E: ExecutorSetBuilder,
    A: AuthorizerBuilder,
{
    async fn build(&mut self, spec: &InstallSpec) -> Result<HostedSession, String> {
        // 1. Fetch the reducer component bytes by content hash. Absent = a clean error (the admin asked to
        //    install a reducer the store doesn't have), not a panic.
        let bytes = self
            .blob
            .get(&spec.reducer_hash)
            .await
            .map_err(|e| {
                format!(
                    "blob store error fetching reducer {}: {e}",
                    spec.reducer_hash
                )
            })?
            .ok_or_else(|| {
                format!(
                    "no reducer component in the blob store for hash {}",
                    spec.reducer_hash
                )
            })?;
        // 2. Lift the bytes into an async reducer. A malformed / non-fold / dep-declaring component is a
        //    clean decline (ComponentError → String), never a panic.
        let reducer = AsyncComponentReducer::from_component_bytes(&bytes).map_err(|e| {
            format!(
                "reducer component for {} did not lift: {e:?}",
                spec.reducer_hash
            )
        })?;
        // 3. Assemble the session. MINT THE NONCE FIRST so we can derive the session's genesis hash BEFORE
        //    building the executor set — a lifecycle executor needs the owner's genesis hash (its spawn
        //    provenance), which isn't the id string (a vanity install id). Then build the session with the
        //    SAME nonce (`genesis_with_nonce`), so its genesis_hash matches what we derived + handed the
        //    executors (a root install has no parent → derive with `None`).
        let spawn_nonce = crate::host::mint_spawn_nonce();
        let owner_genesis =
            cdz_kernel::kernel::Session::derive_genesis_hash(spec.reducer_hash, spawn_nonce, None);
        let executors = self.executors.build(&spec.id, owner_genesis).await;
        let mut hosted = HostedSession::genesis_with_nonce(
            spec.reducer_hash,
            spawn_nonce,
            Box::new(reducer),
            self.authz.build(),
            executors,
        );
        // 4. Attach a per-session durable log if configured ([log].backend = file). A build error (an
        //    un-openable log path) fails the install — a session that should be durable but can't persist
        //    must NOT silently run in-memory-only. `None` = this session keeps its in-memory log (the
        //    `memory` backend).
        if let Some(builder) = &self.log_sink {
            if let Some(sink) = builder.build(&spec.id).await? {
                hosted = hosted.with_sink(sink);
            }
        }
        Ok(hosted)
    }

    async fn build_spawned(
        &mut self,
        reducer_hash: Hash,
        parent_genesis: Hash,
        spawn_nonce: Hash,
    ) -> Result<HostedSession, String> {
        // Same reducer-load path as `build` (blob-get → lift → executor set), but wrapped with
        // parent-provenance + the caller's nonce (so the child's id matches the spawn executor's
        // pre-computation), and a DENY-ALL child authorizer (kernel deny-by-default — a spawned child earns
        // caps via an explicit grant, not by inheriting the parent's policy).
        let bytes = self
            .blob
            .get(&reducer_hash)
            .await
            .map_err(|e| format!("blob store error fetching child reducer {reducer_hash}: {e}"))?
            .ok_or_else(|| {
                format!("no reducer component in the blob store for child hash {reducer_hash}")
            })?;
        let reducer = AsyncComponentReducer::from_component_bytes(&bytes).map_err(|e| {
            format!("child reducer component for {reducer_hash} did not lift: {e:?}")
        })?;
        // The child's own genesis hash, derived from (reducer, nonce, parent) — its SessionId is that hash's
        // hex (the id the loop registers it under) AND its genesis-provenance for the executor set (a spawned
        // child that itself spawns uses this as its parent-genesis).
        let child_genesis = cdz_kernel::kernel::Session::derive_genesis_hash(
            reducer_hash,
            spawn_nonce,
            Some(parent_genesis),
        );
        let child_id = crate::host::SessionId::new(child_genesis.to_hex());
        let executors = self.executors.build(&child_id, child_genesis).await;
        // DENY-ALL child authorizer (NOT self.authz.build() — a spawned child does not inherit the install
        // policy; it starts deny-by-default and earns caps via a later explicit grant).
        let hosted = HostedSession::genesis_spawned_with_nonce(
            reducer_hash,
            spawn_nonce,
            parent_genesis,
            Box::new(reducer),
            Box::new(cdz_kernel::authz::Authorizer::deny_all()),
            executors,
        );
        // NOTE: a per-session durable log sink (the `[log].backend = file` path in `build`) is intentionally
        // NOT attached here — a spawned child's log path would need its child id, which the loop assigns; the
        // in-memory log is correct for the spawn v0 (a durable-log-for-spawned-children slice is a follow-on).
        Ok(hosted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::{AdminCommand, AdminResponse};
    use crate::host::{AgentHost, SessionId};
    use cdz_kernel::authz::Authorizer;
    use cdz_kernel::blob::MemBlobStore;
    use cdz_kernel::hash::Hash;

    /// The reducer-guest fold-world component bytes, if `CDZ_REDUCER_COMPONENT` points at one (CI builds +
    /// exports it; absent locally → the test skips). Distinguishes "var UNSET → skip" from "var SET but the
    /// file is unreadable → PANIC" (#1979 review): a silent `.ok()` on the read would mask a CI misconfig
    /// (the var set to a bad path) as a green skip. If the var is set, the component MUST be readable.
    fn reducer_component_bytes() -> Option<Vec<u8>> {
        let path = std::env::var("CDZ_REDUCER_COMPONENT").ok()?;
        Some(std::fs::read(&path).unwrap_or_else(|e| {
            panic!("CDZ_REDUCER_COMPONENT is set to {path} but the component is unreadable: {e}")
        }))
    }

    fn hermetic_executors() -> CompositeExecutor {
        // Now-only hermetic executor set (no network) — the factory is generic over the set, so a test
        // needs no live-net tree.
        use cdz_kernel::effect::effect_ct;
        CompositeExecutor::new().with_effect(effect_ct::NOW, Box::new(crate::ClockExecutor::new()))
    }

    fn deny_all_authz() -> Box<dyn Authorize> {
        Box::new(Authorizer::deny_all())
    }

    #[tokio::test]
    async fn install_of_an_absent_reducer_hash_is_a_clean_error() {
        // The blob store is empty → get returns None → build errors cleanly (no panic), and apply_admin
        // surfaces it as an AdminResponse::Error with the registry untouched.
        let mut host = AgentHost::new();
        let mut factory =
            ComponentSessionFactory::new(MemBlobStore::new(), hermetic_executors, deny_all_authz);
        let spec = InstallSpec {
            id: SessionId::new("ghost"),
            reducer_hash: Hash::of(b"nonexistent-reducer"),
            goal: None,
        };
        let resp = host
            .apply_admin(AdminCommand::InstallSession(spec), Some(&mut factory), None)
            .await;
        assert!(
            matches!(&resp, AdminResponse::Error { message } if message.contains("no reducer component in the blob store")),
            "absent reducer → clean error: {resp:?}"
        );
        assert!(host.is_empty(), "nothing installed for an absent reducer");
    }

    #[tokio::test]
    async fn install_of_non_component_bytes_is_a_clean_error() {
        // Bytes present but NOT a valid component → from_component_bytes declines → clean error, no panic.
        let mut blob = MemBlobStore::new();
        let garbage = b"this is not a wasm component".to_vec();
        let hash = blob.put(&garbage).await.unwrap();
        let mut factory = ComponentSessionFactory::new(blob, hermetic_executors, deny_all_authz);
        let spec = InstallSpec {
            id: SessionId::new("bad"),
            reducer_hash: hash,
            goal: None,
        };
        let err = match factory.build(&spec).await {
            Ok(_) => panic!("non-component bytes must not lift into a session"),
            Err(e) => e,
        };
        assert!(err.contains("did not lift"), "{err}");
    }

    #[tokio::test]
    async fn install_of_a_real_reducer_component_runs_an_agent() {
        // The end-to-end payoff: a REAL reducer component in the blob store → install builds a live session
        // that a subsequent inbound ACTUALLY DRIVES. Env-gated on a built component fixture (skips locally).
        // The reducer-guest fixture, on a `message` inbound, bumps a `count` KV counter (then requests an
        // Http effect); we deliver one message + assert `count` was written — proving the reducer RAN, not
        // merely that a session registered (#1979 review: the prior version stopped at `contains`).
        let Some(bytes) = reducer_component_bytes() else {
            eprintln!("CDZ_REDUCER_COMPONENT unset — skipping the real-component install test");
            return;
        };
        let mut blob = MemBlobStore::new();
        let hash = blob.put(&bytes).await.unwrap();
        // Grant the session Http so the fold's requested effect authorizes (the KV `count` write happens in
        // the fold regardless, but granting Http keeps the turn from erroring on an AuthzDenied).
        let authz = || -> Box<dyn Authorize> {
            use cdz_kernel::effect::{Capability, EffectKind, ResourcePredicate};
            Box::new(Authorizer::new(vec![Capability {
                kind: EffectKind::Http,
                predicate: ResourcePredicate::Any,
            }]))
        };
        let mut factory = ComponentSessionFactory::new(blob, hermetic_executors, authz);

        let mut host = AgentHost::new();
        let resp = host
            .apply_admin(
                AdminCommand::InstallSession(InstallSpec {
                    id: SessionId::new("real"),
                    reducer_hash: hash,
                    goal: None,
                }),
                Some(&mut factory),
                None,
            )
            .await;
        assert_eq!(
            resp,
            AdminResponse::Installed {
                id: SessionId::new("real")
            },
            "a real reducer component installs"
        );

        // Drive the installed session: deliver a `message` inbound → the reducer's fold runs → `count`=1.
        use cdz_kernel::effect::Payload;
        use cdz_kernel::event::{ContentType, EventBody};
        let outcome = host
            .deliver(
                &SessionId::new("real"),
                EventBody::Inbound {
                    content_type: ContentType {
                        family: "message".into(),
                        version: 1,
                    },
                    payload: Payload::Inline(b"hello".to_vec().into()),
                },
                None,
            )
            .await;
        assert!(
            matches!(outcome, Some(Ok(()))),
            "the installed real reducer ran a turn: {outcome:?}"
        );
        // The reducer WROTE to its own KV during the fold — proof it actually executed (not just registered).
        assert_eq!(
            host.get(&SessionId::new("real"))
                .unwrap()
                .session()
                .kv()
                .get(b"count"),
            Some(&[1u8][..]),
            "the reducer's fold bumped its count counter — the agent ran"
        );
    }

    #[tokio::test]
    async fn file_log_sink_builder_opens_a_per_session_log() {
        // The [log]=file sink builder: build(id) opens a LogStore at <root>/<id>.log and returns Some. A
        // fresh path yields a usable sink + the file exists after open. Unique proven-fresh root (no fixed
        // temp path — #1991 review).
        let root = crate::testutil::unique_temp_dir("filelog");
        let builder = FileLogSinkBuilder::new(&root);
        let sink = builder
            .build(&SessionId::new("sess-1"))
            .await
            .expect("build ok")
            .expect("file backend yields a sink");
        // The sink is a real LogStore; dropping it is fine — the point is the per-session file was created.
        drop(sink);
        assert!(
            root.join("sess-1.log").exists(),
            "per-session log file created at <root>/<id>.log"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn file_log_sink_builder_sanitizes_a_slashed_session_id() {
        // A session id with a '/' must NOT escape the root — it's sanitized to a single filename component.
        // Unique proven-fresh root (#1991 review).
        let root = crate::testutil::unique_temp_dir("filelog-sanitize");
        let builder = FileLogSinkBuilder::new(&root);
        let _sink = builder
            .build(&SessionId::new("evil/../escape"))
            .await
            .expect("build ok")
            .expect("sink");
        // The log stayed under root (the '/'s became '_'), nothing escaped.
        let entries: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            entries,
            vec!["evil_.._escape.log".to_string()],
            "{entries:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A stub executor that returns a SCRIPTED sequence of outcomes (one per `perform`), for exercising the
    /// metering decorator's classification without a real transport.
    struct ScriptedExecutor {
        outcomes: std::cell::RefCell<std::collections::VecDeque<EffectOutcome>>,
    }
    #[async_trait::async_trait(?Send)]
    impl Executor for ScriptedExecutor {
        async fn perform(&mut self, _req: &EffectRequest, _key: Hash) -> EffectOutcome {
            self.outcomes
                .borrow_mut()
                .pop_front()
                .expect("scripted outcome available")
        }
        fn handles_family(&self, family: &str) -> bool {
            family == "scripted"
        }
    }

    #[tokio::test]
    async fn metered_executor_tallies_outcomes_and_passes_them_through() {
        use cdz_kernel::effect::{EffectKind, Payload, Timeliness};

        // Script Ok, a retryable Err, a permanent Err, an err() default (fail-closed → permanent), and a
        // TimedOut (its own counter, not an Err).
        let scripted = ScriptedExecutor {
            outcomes: std::cell::RefCell::new(
                vec![
                    EffectOutcome::Ok(Some(Payload::Inline(b"ok".to_vec().into()))),
                    EffectOutcome::err_retryable("bedrock throttled (429)"),
                    EffectOutcome::err("bad request (400)"),
                    EffectOutcome::err("no token here"),
                    EffectOutcome::TimedOut,
                ]
                .into(),
            ),
        };
        let registry = crate::metrics::Registry::new();
        let metrics = Arc::new(EffectMetrics::new(&registry));
        let mut metered = MeteredExecutor::new(Box::new(scripted), metrics.clone());

        // handles_family forwards (wrapping doesn't hide the served family from the manifest).
        assert!(metered.handles_family("scripted"));
        assert!(!metered.handles_family("other"));

        let req = EffectRequest::new(
            EffectKind::Model,
            "m".to_string(),
            None,
            Timeliness::Interactive,
        );
        // Each outcome passes THROUGH unchanged (the decorator is transparent) — the wrap classifies +
        // records each into the registry along the way (registry Counters are drain-on-report with no value
        // getter, so we assert pass-through + a reportable registry, not per-counter values).
        assert!(matches!(
            metered.perform(&req, Hash::of(b"k")).await,
            EffectOutcome::Ok(_)
        ));
        assert!(matches!(
            metered.perform(&req, Hash::of(b"k")).await,
            EffectOutcome::Err { .. }
        ));
        metered.perform(&req, Hash::of(b"k")).await;
        metered.perform(&req, Hash::of(b"k")).await;
        assert!(matches!(
            metered.perform(&req, Hash::of(b"k")).await,
            EffectOutcome::TimedOut
        ));

        // The registry reports a metrics line over the recorded effect counters (proves the wrap fed the
        // registry — the export path the exporter drives).
        assert!(
            registry.try_take_current_metrics_line().is_some(),
            "the registry reports over the recorded effect metrics"
        );
    }

    #[tokio::test]
    async fn metered_executors_sharing_one_arc_aggregate_across_families() {
        // The wiring pattern LiveExecutorSet::build uses: register MULTIPLE MeteredExecutors (one per family)
        // in a CompositeExecutor, all sharing ONE Arc<EffectMetrics>, so a route to any family tallies into
        // the SAME registry-backed set. Two families here — one Ok + one TimedOut — both recorded through the
        // shared Arc without panic (registry Counters are drain-on-report, no value getter, so we assert the
        // record path is total + the registry reports over it, not per-counter values).
        use cdz_kernel::effect::{EffectKind, Payload, Timeliness};
        use cdz_kernel::executor::Executor as _;

        let registry = crate::metrics::Registry::new();
        let metrics = Arc::new(EffectMetrics::new(&registry));
        let ok_a = ScriptedExecutor {
            outcomes: std::cell::RefCell::new(
                vec![EffectOutcome::Ok(Some(Payload::Inline(
                    b"a".to_vec().into(),
                )))]
                .into(),
            ),
        };
        let timedout_b = ScriptedExecutor {
            outcomes: std::cell::RefCell::new(vec![EffectOutcome::TimedOut].into()),
        };
        let mut composite = CompositeExecutor::new()
            .with_effect(
                "fam-a",
                Box::new(MeteredExecutor::new(Box::new(ok_a), metrics.clone())),
            )
            .with_effect(
                "fam-b",
                Box::new(MeteredExecutor::new(Box::new(timedout_b), metrics.clone())),
            );

        // Route one effect to each family (CompositeExecutor dispatches by req.content_type.family).
        let mut req_a = EffectRequest::new(
            EffectKind::Emit,
            "t".to_string(),
            None,
            Timeliness::Interactive,
        );
        req_a.content_type.family = "fam-a".into();
        let mut req_b = req_a.clone();
        req_b.content_type.family = "fam-b".into();
        composite.perform(&req_a, Hash::of(b"a")).await;
        composite.perform(&req_b, Hash::of(b"b")).await;

        // Both families' outcomes recorded through the one shared Arc into the one registry — which reports
        // over them (proves cross-family aggregation into a single registry-backed set).
        assert!(
            registry.try_take_current_metrics_line().is_some(),
            "the shared registry reports over both families' recorded effects"
        );
    }

    // ─── install_genesis_authorizer (the genesis-completion glue) ───────────────────────────────────────

    use crate::genesis_ct;
    use crate::host::HostedSession;
    use cdz_kernel::event::{Event, EventBody};
    use cdz_kernel::kv::Kv;
    use cdz_kernel::reducer::{FoldOutput, Reducer};

    /// A stand-in genesis reducer (mirrors reducer_genesis.cdz): folds each genesis-setup family's payload
    /// VERBATIM into its contracted KV key.
    struct GenesisRecordingReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for GenesisRecordingReducer {
        async fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
            if let EventBody::Inbound {
                content_type,
                payload: cdz_kernel::effect::Payload::Inline(bytes),
            } = &event.body
            {
                let key: Option<&[u8]> = match content_type.family.as_ref() {
                    genesis_ct::ROOT => Some(genesis_ct::KV_ROOT_IDENTITY),
                    genesis_ct::AUTHORIZER => Some(genesis_ct::KV_AUTHORIZER_HASH),
                    genesis_ct::CONTEXT => Some(genesis_ct::KV_CONTEXT),
                    _ => None,
                };
                if let Some(k) = key {
                    kv.put(k.to_vec(), bytes.to_vec());
                }
            }
            FoldOutput::none()
        }
    }

    fn genesis_session() -> HostedSession {
        HostedSession::genesis(
            Hash::of(b"genesis-reducer-v1"),
            Box::new(GenesisRecordingReducer),
            Box::new(Authorizer::deny_all()),
            hermetic_executors(),
        )
    }

    #[tokio::test]
    async fn install_genesis_authorizer_root_only_is_ok_false() {
        // A root-only genesis records no authorizer-hash → install is a clean Ok(false) (nothing to install,
        // the deny-all authorizer stays), NOT an error.
        let factory =
            ComponentSessionFactory::new(MemBlobStore::new(), hermetic_executors, deny_all_authz);
        let mut session = genesis_session();
        session
            .seed_genesis(b"just-root", None, None)
            .await
            .expect("seed ok");
        let installed = factory
            .install_genesis_authorizer(&mut session, "agent://x")
            .await
            .expect("root-only is a clean Ok");
        assert!(
            !installed,
            "no authorizer-hash recorded → nothing installed"
        );
    }

    #[tokio::test]
    async fn install_genesis_authorizer_bad_length_hash_is_a_clean_err() {
        // A recorded authorizer-hash that isn't 32 bytes is a clean Err (not a panic) — the raw-32 contract is
        // enforced at resolve.
        let factory =
            ComponentSessionFactory::new(MemBlobStore::new(), hermetic_executors, deny_all_authz);
        let mut session = genesis_session();
        session
            .seed_genesis(b"root", Some(b"too-short"), None)
            .await
            .expect("seed ok");
        let err = factory
            .install_genesis_authorizer(&mut session, "agent://x")
            .await
            .expect_err("a non-32-byte hash is rejected");
        assert!(err.contains("32-byte"), "{err}");
    }

    /// A real Cedar policy COMPONENT's bytes, if `CEDAR_POLICY_COMPONENT` points at one (CI builds + exports
    /// it; absent locally → the test skips). Same var-set-but-unreadable → PANIC discipline as
    /// [`reducer_component_bytes`] (#1979): a set-to-bad-path must not read as a green skip.
    fn policy_component_bytes() -> Option<Vec<u8>> {
        let path = std::env::var("CEDAR_POLICY_COMPONENT").ok()?;
        Some(std::fs::read(&path).unwrap_or_else(|e| {
            panic!("CEDAR_POLICY_COMPONENT is set to {path} but the component is unreadable: {e}")
        }))
    }

    #[tokio::test]
    async fn install_genesis_authorizer_happy_path_installs_a_real_policy() {
        // The SUCCESS path (#2184 review): a genesis session records a REAL policy component's hash → install
        // resolves it, lifts it, and installs the authorizer (Ok(true)), flipping the session off deny-all.
        // Env-gated on a built Cedar policy component (skips locally, like the reducer-component test); the
        // resolve/error paths above cover the rest deterministically.
        let Some(bytes) = policy_component_bytes() else {
            eprintln!(
                "CEDAR_POLICY_COMPONENT unset — skipping the real-policy genesis install test"
            );
            return;
        };
        let mut blob = MemBlobStore::new();
        let policy_hash = blob.put(&bytes).await.unwrap();
        let factory = ComponentSessionFactory::new(blob, hermetic_executors, deny_all_authz);

        let mut session = genesis_session();
        // Seed genesis recording the real policy component's hash (raw 32 bytes) as the authorizer pointer.
        session
            .seed_genesis(b"root", Some(policy_hash.as_bytes()), None)
            .await
            .expect("seed ok");

        let installed = factory
            .install_genesis_authorizer(&mut session, "agent://genesis-test")
            .await
            .expect("a real policy component resolves + lifts + installs");
        assert!(installed, "Ok(true): an authorizer was installed");
        // Observable post-condition: the session is intact + renders post-install (the reload swapped the
        // authorizer + ran push_capabilities_changed without erroring). A finer manifest-diff assertion would
        // need the real policy's grant set; this env-gated test proves the resolve→lift→install→push path is
        // total against a real component, complementing the deterministic error-path tests above.
        let status = crate::status::session_status_json(
            &SessionId::new("genesis-test"),
            &session,
            None,
            crate::status::DEFAULT_STALL_AFTER_MS,
        );
        assert!(
            !status.is_empty(),
            "post-install session status renders (authorizer installed, not a broken session)"
        );
    }

    #[tokio::test]
    async fn install_genesis_authorizer_absent_blob_is_a_clean_err() {
        // A valid 32-byte hash that isn't in the blob store → clean Err (the recorded component is missing),
        // no panic. Uses a well-formed 32-byte hash for a component never put into the store.
        let factory =
            ComponentSessionFactory::new(MemBlobStore::new(), hermetic_executors, deny_all_authz);
        let mut session = genesis_session();
        let missing = Hash::of(b"a-policy-component-never-stored");
        session
            .seed_genesis(b"root", Some(missing.as_bytes()), None)
            .await
            .expect("seed ok");
        let err = factory
            .install_genesis_authorizer(&mut session, "agent://x")
            .await
            .expect_err("a hash absent from the blob store is an error");
        assert!(
            err.contains("no authorizer policy component in the blob store"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn run_genesis_ceremony_root_only_seeds_kv_and_returns_ok_false() {
        // The one-call ceremony on a root-only boot: seeds the root into KV + returns Ok(false) (no authorizer
        // recorded → nothing installed, deny-all stays). Proves the wrapper drives seed_genesis then
        // install_genesis_authorizer in one call.
        let factory =
            ComponentSessionFactory::new(MemBlobStore::new(), hermetic_executors, deny_all_authz);
        let mut session = genesis_session();
        let installed = factory
            .run_genesis_ceremony(&mut session, b"root-id", None, None, "agent://x")
            .await
            .expect("root-only ceremony is a clean Ok");
        assert!(!installed, "no authorizer-hash → nothing installed");
        // The seed step ran: the root identity landed in KV.
        assert_eq!(
            session.session().kv().get(genesis_ct::KV_ROOT_IDENTITY),
            Some(&b"root-id"[..])
        );
    }

    #[tokio::test]
    async fn run_genesis_ceremony_propagates_the_install_error() {
        // A ceremony with an authorizer-hash absent from the blob store: seed succeeds, then the install step
        // errors — the wrapper propagates it (a half-booted session is surfaced, not swallowed).
        let factory =
            ComponentSessionFactory::new(MemBlobStore::new(), hermetic_executors, deny_all_authz);
        let mut session = genesis_session();
        let missing = Hash::of(b"policy-never-stored");
        let err = factory
            .run_genesis_ceremony(
                &mut session,
                b"root",
                Some(missing.as_bytes()),
                None,
                "agent://x",
            )
            .await
            .expect_err("the absent-authorizer install error propagates through the ceremony");
        assert!(
            err.contains("no authorizer policy component in the blob store"),
            "{err}"
        );
        // The seed step still ran before the install failed — BOTH the root AND the recorded authorizer-hash
        // are in KV (the half-seeded state the doc warns about: seed is not rolled back on install Err).
        let kv = session.session().kv();
        assert_eq!(kv.get(genesis_ct::KV_ROOT_IDENTITY), Some(&b"root"[..]));
        assert_eq!(
            kv.get(genesis_ct::KV_AUTHORIZER_HASH),
            Some(missing.as_bytes().as_slice()),
            "the authorizer-hash was seeded before the install step failed (half-booted)"
        );
    }
}
