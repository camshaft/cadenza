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
use cdz_kernel::effect::{EffectId, EffectRequest};
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
/// `Ok` → `record_ok`, `Err` → `record_err` split by the outcome's typed
/// [`Retryability`](cdz_kernel::event::Retryability) (the same field a supervisor branches on). Transparent
/// otherwise — the outcome is returned unchanged and
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
    async fn perform(
        &mut self,
        id: EffectId,
        req: &EffectRequest,
        idempotency_key: Hash,
    ) -> EffectOutcome {
        let started = std::time::Instant::now();
        // Forward the kernel-assigned EffectId to the inner executor unchanged (the metering wrapper must be
        // transparent — a delegating inner executor binds its reply-token to this id).
        let outcome = self.inner.perform(id, req, idempotency_key).await;
        self.metrics
            .record_latency_us(crate::metrics::micros_u64(started.elapsed()));
        // Tally the metric + trace at the SAME tap. The schema_hash is the effect identity (STEP C:
        // content_type.family is deleted from EffectRequest in the S3 flip); a failure logs its classified
        // reason (retryable/permanent) at warn (a supervisor signal), ok at debug (routine).
        let schema_hash = req.schema_hash;
        match &outcome {
            EffectOutcome::Ok(_) => {
                self.metrics.record_ok();
                tracing::debug!(target: "cdz_agent_host::effect", ?schema_hash, "effect ok");
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
                    ?schema_hash,
                    retryability = ?retryability,
                    reason = message,
                    "effect errored"
                );
            }
            EffectOutcome::TimedOut => {
                self.metrics.record_timed_out();
                tracing::warn!(target: "cdz_agent_host::effect", ?schema_hash, "effect timed out");
            }
            EffectOutcome::Deferred => {
                // The inner executor DEFERRED (a delegating/userspace-effect executor forwarded the request
                // to a handler session and will settle it later via settle_effect_result). This is NOT a
                // terminal outcome, so tally NOTHING here — the real Ok/Err is recorded when the deferred
                // effect settles + folds back (that settle is a separate path, not this per-perform tap).
                // Trace at debug so the deferral is observable without inflating the ok/err/timed-out counts.
                tracing::debug!(target: "cdz_agent_host::effect", ?schema_hash, "effect deferred (awaiting settle)");
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
    /// The CONTENT-ADDRESSED blob store bucket/prefix for the `blob/put`+`blob/get` executor (cadenza-docs
    /// I3), if the daemon configured an S3 blob backend. When set, `build` wires a per-session
    /// [`BlobExecutor`](crate::BlobExecutor) over a fresh [`S3BlobStore`](crate::S3BlobStore) pointing at this
    /// SAME bucket the reducer components use (one CAS — hash-keyed, so a doc-AST blob + a reducer component
    /// can't collide; corpus-bugfix PM ruling). `None` = no blob executor wired (a build with no S3 blob
    /// config, or without `live-aws-storage`). Behind `live-aws-storage` (the S3 store's feature); the field
    /// exists only when BOTH `live-net` (this struct) AND `live-aws-storage` are compiled.
    #[cfg(feature = "live-aws-storage")]
    blob_store: Option<(String, String)>,
    /// The userspace-effects loop collaborators (design DESIGN-userspace-effects I3/I4), if the daemon wired
    /// them. Set via [`with_userspace_effects`](Self::with_userspace_effects) at boot. When present, `build`
    /// registers, for each installed session: the delegating [`UserspaceEffectExecutor`](crate::UserspaceEffectExecutor)
    /// as the [`CompositeExecutor`] FALLBACK (a dynamic userspace family with a registered handler resolves +
    /// forwards + defers) + the [`ReplyExecutor`](crate::ReplyExecutor) under `effect/reply`. All-or-nothing:
    /// the three collaborators (loop inbox, reply-settle sink, shared reply-token table) come as one wiring the
    /// daemon supplies together; `None` (a pure control-plane / test build) = no userspace-effect delegation is
    /// wired (an unhandled userspace family falls through to the kernel's unhandled-effect path as before).
    /// The live handler resolver is built per-session from the host's shared canonical store, passed to `build`.
    userspace_effects: Option<UserspaceEffectWiring>,
    /// THE OUTPOST federation ws wiring (`live-ws`), if the daemon wired it. Set via [`with_ws`](Self::with_ws)
    /// at boot from the loop's [`AsyncAgentHost::ws_conn_registry`](crate::AsyncAgentHost::ws_conn_registry) +
    /// [`ws_control_channel`](crate::AsyncAgentHost::ws_control_channel) + [`inbox`](crate::AsyncAgentHost::inbox).
    /// When present, `build` wires each installed session's `ws/send` ([`WsSendExecutor`](crate::WsSendExecutor)
    /// over the shared node registry) + `ws/dial` ([`WsDialExecutor`](crate::WsDialExecutor)) so a reducer can
    /// federate. `None` (a build without `live-ws` wiring) = no ws effects wired. The field exists only when
    /// BOTH `live-net` (this struct) AND `live-ws` are compiled — same dual-gate as `blob_store`.
    #[cfg(feature = "live-ws")]
    ws: Option<WsWiring>,
}

/// The boot-time ws collaborators for THE OUTPOST federation (`live-ws`), wired ONCE by the daemon + shared
/// across every session's `build`. The `ws/send` executor sends through `conns` (the ONE node registry the loop
/// drains); the `ws/dial` executor mints a conn-id + spawns the dial, routing `Register`/frames through
/// `control_tx`/`inbox`. Grouped because a federation session needs both the send + dial surface over the same
/// node-scoped connection set.
#[cfg(all(feature = "live-net", feature = "live-ws"))]
#[derive(Clone)]
pub struct WsWiring {
    /// Shared handle to the loop's node connection registry ([`AsyncAgentHost::ws_conn_registry`]) — the
    /// `WsSendExecutor` resolves a conn-id against the SAME map the loop drains `WsControlOp`s into.
    conns: std::rc::Rc<crate::ws_socket::LiveWsConnRegistry>,
    /// The loop's ws-control sender ([`AsyncAgentHost::ws_control_channel`]) — the `WsDialExecutor`'s spawned
    /// dial `Register`s its sink here for the loop to apply.
    control_tx: crate::ws_socket::WsControlSender,
    /// The host loop's inbox — the `WsDialExecutor`'s dial surfaces `ws/connect`/frame/`ws/disconnect` events here.
    inbox: crate::Inbox,
}

/// The boot-time collaborators the userspace-effect executors need, wired ONCE by the daemon and shared across
/// every session's `build` (design DESIGN-userspace-effects I3/I4). Grouped because they are all-or-nothing:
/// the delegating executor forwards into `inbox`, mints into `reply_tokens`, and the reply executor consumes
/// `reply_tokens` + enqueues onto `reply_settle` — a partial set can't route the request→reply→settle loop.
#[cfg(feature = "live-net")]
#[derive(Clone)]
pub struct UserspaceEffectWiring {
    /// The host loop's [`Inbox`](crate::Inbox) sender — the I3 delegating executor forwards an
    /// `effect-request/<family>` [`Inbound`](crate::Inbound) into it (same shape [`EmitExecutor`] uses).
    inbox: crate::Inbox,
    /// The loop's reply-settle sink — the I4 [`ReplyExecutor`](crate::ReplyExecutor) enqueues a
    /// [`ReplySettle`](crate::reply_exec::ReplySettle) here; the loop drains it + settles the caller's effect.
    reply_settle: crate::reply_exec::ReplySettleSink,
    /// The shared one-shot reply-token table (I3 mints on forward, I4 consumes on reply — ONE table across all
    /// sessions on the single-threaded loop, so an `Rc`).
    reply_tokens: std::rc::Rc<crate::ReplyTokenRegistry>,
    /// A clone of the host's shared canonical [`SharedNameStore`](crate::host::SharedNameStore) handle — the
    /// per-session [`CanonicalResolver`](crate::CanonicalResolver) resolves `effect/<family>` LIVE off it
    /// (ruling A). Shared (an `Rc` clone) so a handler registered THIS turn is visible to every session's
    /// fallback immediately.
    canonical: crate::host::SharedNameStore,
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
            #[cfg(feature = "live-aws-storage")]
            blob_store: None,
            userspace_effects: None,
            #[cfg(feature = "live-ws")]
            ws: None,
        })
    }

    /// Wire THE OUTPOST federation ws collaborators (`live-ws`) so each built session gets the `ws/send` +
    /// `ws/dial` effects. Called ONCE at daemon boot with the loop's shared connection registry
    /// ([`AsyncAgentHost::ws_conn_registry`](crate::AsyncAgentHost::ws_conn_registry)), ws-control sender
    /// ([`ws_control_channel`](crate::AsyncAgentHost::ws_control_channel)), and [`inbox`](crate::AsyncAgentHost::inbox).
    /// Without this, a built session serves no ws effects (a non-federating build).
    #[cfg(feature = "live-ws")]
    pub fn with_ws(
        mut self,
        conns: std::rc::Rc<crate::ws_socket::LiveWsConnRegistry>,
        control_tx: crate::ws_socket::WsControlSender,
        inbox: crate::Inbox,
    ) -> Self {
        self.ws = Some(WsWiring {
            conns,
            control_tx,
            inbox,
        });
        self
    }

    /// Wire the userspace-effects loop collaborators (design DESIGN-userspace-effects I3/I4) so each built
    /// session delegates a DYNAMIC userspace effect family to its registered handler. Called ONCE at daemon
    /// boot with the loop's [`Inbox`](crate::Inbox), the reply-settle sink
    /// ([`reply_settle_channel`](crate::reply_exec::reply_settle_channel)), the shared reply-token table, and a
    /// clone of the host's shared canonical store ([`AgentHost::shared_canonical_store`](crate::host::AgentHost::shared_canonical_store)).
    /// Without this, a built session serves no userspace-effect delegation (an unhandled userspace family
    /// falls through to the kernel's unhandled-effect path). The three channel/table collaborators come as one
    /// (all-or-nothing — a partial set can't route request→reply→settle).
    pub fn with_userspace_effects(
        mut self,
        inbox: crate::Inbox,
        reply_settle: crate::reply_exec::ReplySettleSink,
        reply_tokens: std::rc::Rc<crate::ReplyTokenRegistry>,
        canonical: crate::host::SharedNameStore,
    ) -> Self {
        self.userspace_effects = Some(UserspaceEffectWiring {
            inbox,
            reply_settle,
            reply_tokens,
            canonical,
        });
        self
    }

    /// Wire the CONTENT-ADDRESSED blob store (bucket + key prefix) for the `blob/put`+`blob/get` executor
    /// (cadenza-docs I3). Called ONCE at daemon boot when the `[blob]` backend is S3, with the SAME
    /// bucket/prefix the reducer-component store uses (one CAS — corpus-bugfix PM ruling: docs + components
    /// coexist by hash, no separate doc bucket). Each `build` then constructs a fresh per-session
    /// [`S3BlobStore`](crate::S3BlobStore) over this bucket (a stateless client keyed to the bucket — cheap
    /// clone, like the log sink) + wires [`BlobExecutor`](crate::BlobExecutor) under `blob/put`+`blob/get`,
    /// Cedar-gated (the `memory/`-write grant governs which docs a session may publish). Without this, a built
    /// session serves no blob effects.
    #[cfg(feature = "live-aws-storage")]
    pub fn with_blob_store(mut self, bucket: impl Into<String>, prefix: impl Into<String>) -> Self {
        self.blob_store = Some((bucket.into(), prefix.into()));
        self
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
        // NOTE (GAP-6 REVERSED by operator 2026-08-14): git is a USERSPACE shell-wrapper (reducer-side
        // composition of the generic `shell` effect, evolvable without host redeploys), NOT a host git
        // executor — so the former `live-git` GitExecutor wiring here was removed (full-collapse). No host-side
        // git effect family / arg surface exists; a session runs git via the shell effect its Cedar policy
        // permits, composed in the reducer (v-agent-harness role-library lane).
        // §lifecycle (tick-#784 gap): when the daemon wired the loop's lifecycle channel, register the four
        // `lifecycle/*` executors so an installed session can spawn/suspend/resume/terminate. Each is owned
        // by THIS session's id (the `by` stamped on its lifecycle ops); the LifecycleExecutor records a
        // LifecycleOp on the channel and the loop applies it (defer-to-loop). The CompositeExecutor routes by
        // EXACT family, so one executor per verb (each holds a clone of the channel + the owner id). NO host
        // policy — whether a session MAY lifecycle a target is the Cedar authority (I6 DescendantOf), gated
        // before dispatch. `None` channel (control-plane/test build) = no lifecycle effect served.
        let composite = if let Some(lifecycle) = &self.lifecycle {
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
                            *id,
                            owner_genesis,
                        )),
                        m.clone(),
                    )),
                );
            }
            c
        } else {
            composite
        };
        // I3: wire the `blob/put`+`blob/get` executor when an S3 blob store is configured — a per-session
        // S3BlobStore over the SAME content-addressed bucket the reducer components use (one CAS; corpus-bugfix
        // PM ruling). Cedar gates WHICH blobs a session may put/get on the effect's resolved target (the
        // `memory/`-write grant for docs), so wiring the mechanism for all is safe — same posture as shell/fs.
        // The CompositeExecutor routes by EXACT family, so register the (unit-generic) BlobExecutor under each
        // blob verb. NOT metered (the blob executor holds a store, not a leaf transport — a following slice can
        // add metering; the store's own errors already fold as classified EffectOutcome::Err).
        #[cfg(feature = "live-aws-storage")]
        let composite = {
            let mut c = composite;
            if let Some((bucket, prefix)) = &self.blob_store {
                for fam in [effect_ct::BLOB_PUT, effect_ct::BLOB_GET] {
                    c = c.with_effect(
                        fam,
                        Box::new(crate::BlobExecutor::new(
                            crate::S3BlobStore::new(bucket.clone(), prefix.clone()).await,
                        )),
                    );
                }
            }
            c
        };
        // THE OUTPOST federation: wire `ws/send` + `ws/dial` when the daemon wired the ws collaborators
        // (`live-ws`). ws/send routes a frame to a live conn through the SHARED node registry (the SAME map the
        // loop drains WsControlOps into — an Rc clone, so this session sends over the node's connections);
        // ws/dial mints a conn-id, spawns dial_hub, returns the id (reducer-emittable federation dial). Cedar
        // gates WHICH hubs/conns a session may dial/send on the effect target (egress/SSRF guard), so wiring the
        // mechanism for all is safe — same posture as shell/blob. Both metered.
        #[cfg(feature = "live-ws")]
        let composite = {
            let mut c = composite;
            if let Some(ws) = &self.ws {
                c = c
                    .with_effect(
                        effect_ct::WS_SEND,
                        Box::new(MeteredExecutor::new(
                            Box::new(crate::WsSendExecutor::new(std::rc::Rc::clone(&ws.conns))),
                            m.clone(),
                        )),
                    )
                    .with_effect(
                        effect_ct::WS_DIAL,
                        Box::new(MeteredExecutor::new(
                            Box::new(crate::WsDialExecutor::new(
                                ws.inbox.clone(),
                                ws.control_tx.clone(),
                                *id,
                            )),
                            m.clone(),
                        )),
                    );
            }
            c
        };
        // Userspace-effects I3/I4: when the daemon wired the loop collaborators, register for this session
        // (a) the ReplyExecutor under `effect/reply` (a handler's reply verb → validate+consume the token →
        // enqueue a ReplySettle the loop folds onto the caller), and (b) the UserspaceEffectExecutor as the
        // composite FALLBACK — a DYNAMIC userspace family (no built-in executor) with a registered handler
        // resolves LIVE off the shared canonical store (CanonicalResolver, ruling A), forwards the request to
        // the handler + DEFERS. The fallback SELF-GUARDS (is_registered_effect_family && a handler resolves),
        // so a genuinely-unhandled family still returns its own PERMANENT Err = the kernel's anti-stuck
        // no-executor behavior is preserved. `id` is this session's owner (the caller a forward binds its
        // reply-token to). Not metered: the delegating executor forwards + defers (no leaf outcome to tally);
        // the eventual settle folds the handler's real outcome.
        if let Some(ue) = &self.userspace_effects {
            let resolver = crate::CanonicalResolver::new(ue.canonical.clone());
            composite
                .with_effect(
                    effect_ct::EFFECT_REPLY,
                    Box::new(crate::ReplyExecutor::new(
                        ue.reply_tokens.clone(),
                        ue.reply_settle.clone(),
                    )),
                )
                .with_fallback(Box::new(crate::UserspaceEffectExecutor::new(
                    resolver,
                    ue.inbox.clone(),
                    ue.reply_tokens.clone(),
                    *id,
                )))
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

    /// Read back a session's durable log for BOOT-RECOVERY (§lifecycle I4b): the recovery counterpart to
    /// [`build`](Self::build). Where `build` opens a sink to APPEND a live session's events, this reads the
    /// PERSISTED events for `id` into a [`Recovered`](cdz_kernel::log_store::Recovered) the daemon feeds to
    /// [`Session::recover_from`](cdz_kernel::kernel::Session::recover_from). `Ok(None)` = this backend has no
    /// durable log to recover from (the `memory` backend — nothing persisted, so nothing to recover);
    /// `Ok(Some(recovered))` carries the events + a recovery kind (Clean / a Corrupt-or-torn prefix the loop
    /// alarms on but still recovers the good prefix of); `Err` = the read itself failed (an I/O / backend
    /// error, distinct from an absent log).
    ///
    /// DEFAULT: `Ok(None)` — a builder with no recovery-read (a test stub, or a backend that doesn't persist)
    /// declines cleanly, so the boot-recovery loop skips it rather than the trait forcing every impl to grow a
    /// read path. The durable [`FileLogSinkBuilder`] / `DynamoLogSinkBuilder` override it.
    async fn recover(
        &self,
        _id: &crate::host::SessionId,
    ) -> Result<Option<cdz_kernel::log_store::Recovered>, String> {
        Ok(None)
    }

    /// Load the KV SNAPSHOT a checkpoint descriptor's `kv_root` names (GAP-4 D3) — the recovery counterpart to
    /// the checkpoint WRITE ([`FileLogSinkBuilder::checkpoint`]). A checkpoint-rooted durable log
    /// (`[Genesis, Checkpoint@N, tail]`) recovers via
    /// [`Session::recover_from_checkpoint`](cdz_kernel::kernel::Session::recover_from_checkpoint), which needs
    /// the KV CONTENT the descriptor's `kv_root` addresses (the durable frame carries only the root hash). The
    /// snapshot lives in this backend's checkpoint/offload store, so the recovery path resolves it through the
    /// SAME builder that wrote it. `Ok(Some(kv))` present (its `root_hash()` == `kv_root`); `Ok(None)` absent
    /// (no snapshot store configured, or the snapshot the checkpoint names isn't in the store — the caller
    /// falls back to full-log replay); `Err` a store I/O error or a corrupt snapshot (never a panic).
    ///
    /// DEFAULT: `Ok(None)` — a backend with no checkpoint-snapshot store (the `memory` backend, or a file log
    /// with no offload configured) never produces checkpoint-rooted logs, so it has nothing to load. The
    /// durable [`FileLogSinkBuilder`] (with an offload store) overrides it.
    async fn load_checkpoint_snapshot(
        &self,
        _kv_root: cdz_kernel::hash::Hash,
    ) -> Result<Option<cdz_kernel::kv::Kv>, String> {
        Ok(None)
    }

    /// GAP-4 D3-4 checkpoint WRITE — compact this session's durable log to `[Genesis, Checkpoint@N, tail]` +
    /// persist the KV snapshot. The dyn-dispatchable seam the per-turn trigger
    /// ([`HostedSession::maybe_checkpoint`](crate::host::HostedSession::maybe_checkpoint)) calls when the
    /// [`CheckpointPolicy`](crate::checkpoint::CheckpointPolicy) fires — a session holds its builder as
    /// `Box<dyn LogSinkBuilder>`, so checkpoint must be on the trait, not just an inherent method.
    ///
    /// DEFAULT: unsupported — a backend with no checkpoint store (the `memory` backend, or a file log with no
    /// offload) never checkpoints. The durable [`FileLogSinkBuilder`] (with an offload store) overrides it.
    async fn checkpoint(
        &self,
        _id: &crate::host::SessionId,
        _session: &cdz_kernel::kernel::Session,
    ) -> Result<(), String> {
        Err("checkpoint: this log backend does not support checkpointing".to_string())
    }
}

/// Where a GAP-4 D1 log-body OFFLOAD store is rooted — the content-addressed backing a `[log]` offload writes
/// over-threshold event bodies to (and rehydrates from on recovery). Mirrors the `[blob]` effect-store
/// backends so the log offload reuses the SAME physical CAS: `Dir` (a local [`DiskBlobStore`]) or `S3` (the
/// prod pairing with a Dynamo/File log). [`materialize`](Self::materialize)d to a FRESH raw `Box<dyn BlobStore>`
/// per `build`/`recover` — `put` is `&mut self`, so each per-session sink owns its own handle over the one
/// physical store, not a shared object. RAW backing, NOT the read-cache wrapper: a log-body offload put must
/// write-through to be as durable as the log frame that points at it (`recover_rehydrated` treats an absent
/// CAS body as a HARD io error). `Clone` (an `SdkConfig` is a cheap handle) so the daemon can hand the same
/// source to a session's append builder AND its recovery reader.
// `large_enum_variant`: the S3 variant (an SdkConfig handle + two strings) is larger than Dir, but an
// OffloadSource is a CONFIG-time value built once per `[log]` backend and cloned a handful of times per
// session build — never held in bulk or on a hot path — so boxing the variant would only add a pointless
// heap indirection. The size asymmetry is immaterial here.
#[allow(clippy::large_enum_variant)]
#[derive(Clone)]
pub enum OffloadSource {
    /// A local content-addressed [`DiskBlobStore`](cdz_kernel::blob::DiskBlobStore) rooted at this dir (the
    /// SAME `[blob]` dir the effect path uses — one physical CAS).
    Dir(std::path::PathBuf),
    /// An [`S3BlobStore`](crate::S3BlobStore) over `(bucket, prefix)` — the prod pairing with a Dynamo/File
    /// log. Holds the ambient `SdkConfig` (loaded ONCE on the daemon boot path) so per-session materialization
    /// is a cheap client clone via `S3BlobStore::from_conf`, not a fresh IMDS probe.
    #[cfg(feature = "live-aws-storage")]
    S3 {
        config: aws_config::SdkConfig,
        bucket: String,
        prefix: String,
    },
}

impl OffloadSource {
    /// Open a FRESH raw `Box<dyn BlobStore>` over this source — a separate handle over the one physical CAS
    /// (`put` is `&mut self`, so each `build`/`recover` owns its handle). Raw backing, no read cache: an
    /// offloaded log body must write-through to be as durable as the log frame that points at it.
    pub(crate) fn materialize(&self) -> Result<Box<dyn cdz_kernel::blob::BlobStore>, String> {
        match self {
            OffloadSource::Dir(dir) => cdz_kernel::blob::DiskBlobStore::open(dir)
                .map(|s| Box::new(s) as Box<dyn cdz_kernel::blob::BlobStore>)
                .map_err(|e| {
                    format!(
                        "could not open log-offload blob store {}: {e}",
                        dir.display()
                    )
                }),
            #[cfg(feature = "live-aws-storage")]
            OffloadSource::S3 {
                config,
                bucket,
                prefix,
            } => Ok(Box::new(crate::S3BlobStore::from_conf(
                config,
                bucket.clone(),
                prefix.clone(),
            ))),
        }
    }
}

/// A [`LogSinkBuilder`] that opens a per-session file-backed [`LogStore`](cdz_kernel::log_store::LogStore)
/// under a root directory — the `[log].backend = file` durable log. Each session's log is
/// `<root>/<session-id>.log`; the session id is sanitized (path separators → `_`) so an id can't escape the
/// root. The deployed daemon installs this when the log config selects a file backend.
pub struct FileLogSinkBuilder {
    root: std::path::PathBuf,
    /// GAP-4 D1 log-body OFFLOAD (opt-in): when `Some((source, threshold))`, a persisted event body larger
    /// than `threshold` is offloaded to the content-addressed [`OffloadSource`] (a tiny `(blob-ptr)` frame
    /// replaces the body in the log), and boot-recovery rehydrates it. `None` = pre-D1 behavior (bodies stay
    /// inline). A fresh raw handle over the SAME `[blob]` store is opened per `build`/`recover`.
    offload: Option<(OffloadSource, usize)>,
}

impl FileLogSinkBuilder {
    /// Root the per-session logs at `root` (created on first open by `LogStore::open`). No body offload.
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        FileLogSinkBuilder {
            root: root.into(),
            offload: None,
        }
    }

    /// Enable GAP-4 D1 log-body offload: bodies over `threshold` bytes offload to the content-addressed
    /// [`OffloadSource`] (the SAME `[blob]` store the effect-blob path uses — one physical CAS). Recovery
    /// rehydrates. The daemon calls this when `[log].backend = file` AND `[log].offload_threshold` is set,
    /// passing the `[blob]`-derived source (Dir or S3).
    pub fn with_offload(mut self, source: OffloadSource, threshold: usize) -> Self {
        self.offload = Some((source, threshold));
        self
    }

    /// The per-session log path for `id`: `<root>/<id-hex>.log`. The id is a genesis [`Hash`] rendered as hex
    /// (`[0-9a-f]{64}`), which is a single safe filename component by construction — it holds no path
    /// separator, so no crafted id can escape the root. `build` (append) and `recover` (read-back) MUST agree
    /// on this path, so both go through here.
    fn log_path(&self, id: &crate::host::SessionId) -> std::path::PathBuf {
        self.root.join(format!("{}.log", id.to_hex()))
    }

    /// GAP-4 D3-2: crash-safely CHECKPOINT session `id`'s durable file log — the host half of
    /// log-prune-to-checkpoint. Persists `session`'s KV snapshot to the configured offload store (content-
    /// addressed by its root hash = the checkpoint descriptor's `kv_root`), then REWRITES the log to
    /// `[Genesis, Checkpoint@N, tail(seq > N)]` — dropping the subsumed `1..=N` prefix
    /// ([`Session::checkpoint_subsumed_prefix`](cdz_kernel::kernel::Session::checkpoint_subsumed_prefix)) — via
    /// a temp file + atomic rename.
    ///
    /// **Crash-safe (safety condition 5)** by never appending the checkpoint frame to the LIVE full log: the
    /// snapshot is persisted FIRST, the compacted log is built in a temp file, fsync'd, then atomically renamed
    /// over the original. A crash BEFORE the rename recovers to the intact full log (a normal
    /// [`Session::recover_from`](cdz_kernel::kernel::Session::recover_from) full replay — no checkpoint frame is
    /// present); a crash AFTER recovers to the compacted log ([`Session::recover_from_checkpoint`](cdz_kernel::kernel::Session::recover_from_checkpoint));
    /// never a torn mix. The `Checkpoint` frame therefore only ever exists in the post-rename compacted log.
    ///
    /// Requires an offload store configured ([`with_offload`](Self::with_offload)) — the snapshot goes to the
    /// SAME content-addressed store D1 body-offload uses. `Err` if none is set, if the log has no genesis at
    /// `events[0]`, or on any store/IO failure (never a panic). NOTE: nothing TRIGGERS this yet (the checkpoint
    /// trigger policy is a later slice); a session's log is only ever rewritten by an explicit call.
    pub async fn checkpoint(
        &self,
        id: &crate::host::SessionId,
        session: &cdz_kernel::kernel::Session,
    ) -> Result<(), String> {
        use cdz_kernel::log_store::LogStore;
        let (source, _threshold) = self.offload.as_ref().ok_or_else(|| {
            "checkpoint requires an offload store (no [blob]/offload configured for this file log)".to_string()
        })?;
        // A fresh handle over the content-addressed store — `put` (persist snapshot) needs `&mut`, the
        // subsequent rehydrating read needs `&`; one handle serves both.
        let mut blob = source.materialize()?;

        // The checkpoint frame sits at the session's CURRENT tip (seq N) and carries the full descriptor
        // (incl. `kv_root` = session.kv().root_hash()). Snapshot FIRST (crash-safe order): persist the KV
        // content the descriptor's `kv_root` names, so a post-rename recovery always finds its seed_kv.
        let frame = session.checkpoint_frame();
        let checkpoint_seq = frame.seq;
        crate::checkpoint::persist_kv_snapshot(blob.as_mut(), session.kv()).await?;

        // Read the current durable frames (rehydrating any D1-offloaded bodies so the rewrite carries full
        // bodies). Genesis (events[0]) MUST be present — it holds the identity/reducer/provenance recovery
        // reads, and is ALWAYS kept.
        let path = self.log_path(id);
        let recovered = LogStore::recover_rehydrated(&path, blob.as_ref())
            .await
            .map_err(|e| format!("checkpoint: could not read log {}: {e}", path.display()))?;
        let genesis = recovered
            .events
            .first()
            .filter(|e| matches!(e.body, cdz_kernel::event::EventBody::Genesis { .. }))
            .cloned()
            .ok_or_else(|| {
                format!(
                    "checkpoint: log {} has no genesis event at events[0]",
                    path.display()
                )
            })?;

        // Compacted log = [Genesis, Checkpoint@N, tail (seq > N)]. The subsumed prefix (1..=N) is dropped; the
        // checkpoint frame subsumes it. (The tail is normally empty — the checkpoint is at the current tip —
        // but the filter is general so a checkpoint of an older seq keeps any later frames.)
        let mut compacted: Vec<cdz_kernel::event::Event> = Vec::with_capacity(2);
        compacted.push(genesis);
        compacted.push(frame);
        compacted.extend(
            recovered
                .events
                .into_iter()
                .filter(|e| e.seq > checkpoint_seq),
        );

        // Write the compacted log to a TEMP file, fsync its bytes, then ATOMIC-rename over the original — so a
        // crash leaves either the intact original or the complete compacted log, never a torn frame.
        let tmp = path.with_extension("checkpointing");
        {
            let mut store = LogStore::open(&tmp).map_err(|e| {
                format!("checkpoint: could not open temp log {}: {e}", tmp.display())
            })?;
            for e in &compacted {
                store
                    .append(e)
                    .map_err(|e2| format!("checkpoint: temp log append failed: {e2}"))?;
            }
        } // drop closes the temp file's handle before we fsync + rename it
        std::fs::File::open(&tmp)
            .and_then(|f| f.sync_all())
            .map_err(|e| {
                format!(
                    "checkpoint: could not fsync temp log {}: {e}",
                    tmp.display()
                )
            })?;
        std::fs::rename(&tmp, &path).map_err(|e| {
            format!(
                "checkpoint: atomic rename of {} failed: {e}",
                path.display()
            )
        })?;
        // Flush the DIRECTORY entry the rename created (reviewer durability catch on D3-2a): `rename` durably
        // swaps the file CONTENT, but the parent-directory metadata recording the new name is not guaranteed on
        // disk until the directory itself is fsync'd — a crash between the rename and that metadata flush could
        // revert to the pre-rename entry (a silently-lost checkpoint = log-growth regression once checkpointing
        // is live). Open the parent dir and sync_all it. Surfaced as an Err on a real fsync failure (never a
        // panic); a path with no parent component is a no-op.
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::File::open(parent)
                .and_then(|d| d.sync_all())
                .map_err(|e| {
                    format!(
                        "checkpoint: could not fsync log directory {} after rename: {e}",
                        parent.display()
                    )
                })?;
        }
        Ok(())
    }
}

#[async_trait::async_trait(?Send)]
impl LogSinkBuilder for FileLogSinkBuilder {
    async fn build(
        &self,
        id: &crate::host::SessionId,
    ) -> Result<Option<Box<dyn cdz_kernel::log_store::LogSink>>, String> {
        // Ensure the root dir exists (LogStore::open opens a file, it does NOT create parent dirs — unlike
        // DiskBlobStore::open). Create it once here so a fresh [log].dir just works.
        std::fs::create_dir_all(&self.root)
            .map_err(|e| format!("could not create log dir {}: {e}", self.root.display()))?;
        let path = self.log_path(id);
        let store = cdz_kernel::log_store::LogStore::open(&path)
            .map_err(|e| format!("could not open durable log {}: {e}", path.display()))?;
        // GAP-4 D1: attach body-offload when configured — a fresh raw DiskBlobStore handle over the shared
        // `[blob]` CAS root (own handle per build, `put` is `&mut self`; raw backing for write-through
        // durability, not the read cache).
        let store = match &self.offload {
            Some((source, threshold)) => store.with_offload(source.materialize()?, *threshold),
            None => store,
        };
        Ok(Some(Box::new(store)))
    }

    /// Read back this session's file log for boot-recovery. A MISSING file = `Ok(None)` (no such session
    /// persisted — the daemon skips it), NOT an error; the file backend recovers a real on-disk log via the
    /// kernel's `LogStore::recover` (which reports Clean / a torn-tail good-prefix). A genuine read/parse I/O
    /// failure is `Err`.
    async fn recover(
        &self,
        id: &crate::host::SessionId,
    ) -> Result<Option<cdz_kernel::log_store::Recovered>, String> {
        let path = self.log_path(id);
        if !path.exists() {
            return Ok(None);
        }
        // GAP-4 D1: rehydrate offloaded bodies on recovery when offload is configured — a `(blob-ptr)` frame
        // derefs its body from the same CAS root (`recover_rehydrated` takes `&blob`, get-only, so a borrow of
        // a fresh raw handle suffices). Without offload, the plain sync `recover` (no blob) is the hot path.
        match &self.offload {
            Some((source, _threshold)) => {
                let blob = source.materialize()?;
                cdz_kernel::log_store::LogStore::recover_rehydrated(&path, blob.as_ref())
                    .await
                    .map(Some)
                    .map_err(|e| format!("could not recover durable log {}: {e}", path.display()))
            }
            None => cdz_kernel::log_store::LogStore::recover(&path)
                .map(Some)
                .map_err(|e| format!("could not recover durable log {}: {e}", path.display())),
        }
    }

    /// Load a checkpoint's KV snapshot from the offload store this builder writes checkpoints to (GAP-4 D3).
    /// The snapshot was persisted by [`checkpoint`](Self::checkpoint) content-addressed by its `kv_root`, so a
    /// fresh handle over the SAME [`OffloadSource`] resolves it by that key. No offload configured → this file
    /// log never checkpoints → `Ok(None)` (nothing to load). Delegates the get+decode to
    /// [`crate::checkpoint::load_kv_snapshot`] (absent → `None`; corrupt → `Err`).
    async fn load_checkpoint_snapshot(
        &self,
        kv_root: cdz_kernel::hash::Hash,
    ) -> Result<Option<cdz_kernel::kv::Kv>, String> {
        let Some((source, _threshold)) = &self.offload else {
            return Ok(None);
        };
        let blob = source.materialize()?;
        crate::checkpoint::load_kv_snapshot(blob.as_ref(), kv_root).await
    }

    /// GAP-4 D3-4: the trait-dispatchable checkpoint override — delegates to the inherent
    /// [`FileLogSinkBuilder::checkpoint`] (the crash-safe compact-rewrite). UFCS names the inherent method
    /// explicitly (no recursion) so a session holding this builder as `Box<dyn LogSinkBuilder>` can checkpoint
    /// its own log via the trait.
    async fn checkpoint(
        &self,
        id: &crate::host::SessionId,
        session: &cdz_kernel::kernel::Session,
    ) -> Result<(), String> {
        FileLogSinkBuilder::checkpoint(self, id, session).await
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
    /// The per-session durable-log builder, held as an `Rc` so it can be SHARED into each built session's
    /// checkpoint capability (GAP-4 D3-4) via a cheap `Rc::clone` — the builder appends AND (per-session)
    /// checkpoint-rewrites through the same handle. `Rc` (not `Arc`): the daemon loop is single-threaded.
    log_sink: Option<std::rc::Rc<dyn LogSinkBuilder>>,
    /// GAP-4 D3-4 checkpoint TRIGGER policy applied to every durably-logged session this factory builds
    /// (from `[log].checkpoint_threshold`). Default DISABLED = no checkpointing (unbounded log). Set via
    /// [`with_checkpoint_policy`](Self::with_checkpoint_policy).
    checkpoint_policy: crate::checkpoint::CheckpointPolicy,
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
            checkpoint_policy: crate::checkpoint::CheckpointPolicy::DISABLED,
        }
    }

    /// Attach a per-session durable-log [`LogSinkBuilder`] — the deployed daemon supplies one when
    /// `[log].backend = file` so each installed session's event log persists to disk. Builder-style. Takes a
    /// `Box` (the daemon's natural handle) and wraps it in an `Rc` so it can be shared into each session's
    /// checkpoint capability.
    pub fn with_log_sink(mut self, log_sink: Box<dyn LogSinkBuilder>) -> Self {
        self.log_sink = Some(std::rc::Rc::from(log_sink));
        self
    }

    /// Set the GAP-4 D3-4 checkpoint TRIGGER policy applied to every durably-logged session this factory
    /// builds (the daemon derives it from `[log].checkpoint_threshold`). Only takes effect for sessions that
    /// also get a durable log sink ([`with_log_sink`](Self::with_log_sink)); a `memory`-backed session never
    /// checkpoints regardless. Builder-style; default (unset) is [`CheckpointPolicy::DISABLED`](crate::checkpoint::CheckpointPolicy::DISABLED).
    pub fn with_checkpoint_policy(mut self, policy: crate::checkpoint::CheckpointPolicy) -> Self {
        self.checkpoint_policy = policy;
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
                // `get` now returns `Bytes` (KV-Bytes, operator seq364); a `[u8; 32]` converts from a slice,
                // so borrow the Bytes as a slice for the length-checked try_into (no leaf .to_vec()).
                let arr: [u8; 32] = b.as_ref().try_into().map_err(|_| {
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
            // GAP-4 D3-4: attach the per-turn checkpoint capability — the SAME builder (a shared `Rc`) so the
            // session can compact-rewrite its own durable log + rebuild the sink when the policy fires. A
            // DISABLED policy (the default) makes this inert, so this is safe to attach unconditionally when a
            // durable sink exists; only a configured `[log].checkpoint_threshold` actually triggers.
            hosted = hosted.with_checkpoint(builder.clone(), self.checkpoint_policy);
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
        let child_id = crate::host::SessionId::new(child_genesis);
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

    /// Rebuild a HostedSession from an ALREADY-RECOVERED kernel [`Session`] (§lifecycle I4b boot-recovery) —
    /// the recovery counterpart to [`build`](Self::build). Where `build` MINTS a fresh session from an
    /// [`InstallSpec`], this wraps the `Session` that [`Session::recover_from`](cdz_kernel::kernel::Session::recover_from)
    /// rebuilt from a durable log: it reloads the session's REDUCER (by the hash in its own genesis event —
    /// read host-side off `session.log()[0]`, no kernel change) + a fresh executor set, then
    /// [`HostedSession::from_recovered`](crate::host::HostedSession::from_recovered) wraps them (the recovered
    /// session keeps its genesis-hash / SessionId / KV / open obligations — recovery never re-mints).
    ///
    /// A CHECKPOINT-ROOTED log (GAP-4 D3: `[Genesis, Checkpoint@N, tail]`, written by
    /// [`FileLogSinkBuilder::checkpoint`]) is fast-forwarded instead: the seed-KV snapshot the checkpoint
    /// descriptor names is loaded from this backend's checkpoint store ([`load_checkpoint_snapshot`](LogSinkBuilder::load_checkpoint_snapshot))
    /// and the tail folded via [`Session::recover_from_checkpoint`](cdz_kernel::kernel::Session::recover_from_checkpoint) —
    /// recovery-equivalent to full replay, without re-folding the subsumed prefix. A plain genesis-rooted log
    /// takes the full-replay path.
    ///
    /// The boot-recovery loop calls this per durably-logged session (enumerated via the session-registry /
    /// `DynamoSessionRegistry::list_all`), after skipping any [`is_terminated`](crate::host::HostedSession::is_terminated).
    /// A DENY-ALL authorizer is attached (like a spawned child): a recovered session re-earns caps via its
    /// replayed authorizer-install or a fresh grant, not by trusting a rebuilt-from-nothing policy.
    ///
    /// Errors (never a panic): the recovered log has no genesis event (`log[0]` not `Genesis` — a corrupt
    /// recovery the caller alarms on), the reducer bytes are absent from the blob store (the reducer the
    /// session ran isn't available — can't rebuild it), or the component doesn't lift.
    async fn recover_and_build(
        &mut self,
        recovered: cdz_kernel::log_store::Recovered,
    ) -> Result<(HostedSession, cdz_kernel::kernel::RecoveryReport), String> {
        use cdz_kernel::event::EventBody;
        // Read the reducer hash from the recovered log's OWN genesis event (events[0]). The host reads it
        // directly (EventBody::Genesis is pub with pub fields) — no kernel accessor needed. recover_from
        // requires a genesis at events[0]; guard so an empty / genesis-less log is a clean error, not a panic.
        let reducer_hash = match recovered.events.first().map(|e| &e.body) {
            Some(EventBody::Genesis { reducer, .. }) => *reducer,
            _ => {
                return Err(
                    "recover_and_build: the recovered log has no genesis event at events[0]".into(),
                )
            }
        };
        // GAP-4 D3-2b: a CHECKPOINT-ROOTED durable log ([Genesis@0, Checkpoint@N, tail]) recovers by
        // FAST-FORWARDING from the KV SNAPSHOT the checkpoint descriptor names, then folding only the tail —
        // NOT a full replay from genesis (the subsumed 1..=N prefix is gone, and a Checkpoint frame is a
        // recovery SEED, not a foldable event). Detect the Checkpoint at events[1] and resolve its seed-KV
        // snapshot from THIS backend's checkpoint store (the same builder that wrote it, via
        // `load_checkpoint_snapshot`) BEFORE the reducer fetch: a checkpoint-rooted log with no snapshot store
        // configured, or one naming a `kv_root` that isn't present, is a clean Err here rather than a needless
        // component fetch followed by an unfoldable-Checkpoint replay. A plain full log resolves to `None` and
        // takes the existing genesis-rooted replay path unchanged.
        let seed_kv = match recovered.events.get(1).map(|e| &e.body) {
            Some(EventBody::Checkpoint(desc)) => {
                let kv_root = desc.kv_root;
                let builder = self.log_sink.as_ref().ok_or_else(|| {
                    format!(
                        "recover_and_build: checkpoint-rooted log (Checkpoint at events[1]) but no log \
                         sink is configured to load the seed KV snapshot for kv_root {kv_root}"
                    )
                })?;
                let kv = builder
                    .load_checkpoint_snapshot(kv_root)
                    .await?
                    .ok_or_else(|| {
                        format!(
                            "recover_and_build: checkpoint-rooted log names kv_root {kv_root} but no \
                             matching KV snapshot is in the checkpoint store"
                        )
                    })?;
                Some(kv)
            }
            _ => None,
        };
        let bytes = self
            .blob
            .get(&reducer_hash)
            .await
            .map_err(|e| {
                format!("blob store error fetching recovered reducer {reducer_hash}: {e}")
            })?
            .ok_or_else(|| {
                format!("no reducer component in the blob store for recovered hash {reducer_hash}")
            })?;
        let mut reducer = AsyncComponentReducer::from_component_bytes(&bytes).map_err(|e| {
            format!("recovered reducer component for {reducer_hash} did not lift: {e:?}")
        })?;
        // Fold the recovered log with the reloaded reducer: a checkpoint-rooted log seeds the resident state
        // from the snapshot at seq N and folds only the tail (recovery-EQUIVALENT to full replay by
        // construction — kernel-proven in `recover_from_checkpoint_equals_full_replay`); a full log replays
        // from genesis through the backend-agnostic recovery core. Either way it rebuilds KV + the
        // open-obligation set and reports how the log ended (Clean/TornTail/Corrupt) + which effects are open
        // (the daemon re-drives those). `recover_from_checkpoint` returns only a `Session` (no report), so the
        // checkpoint path constructs the equivalent `RecoveryReport` from the recovered `kind` + open effects.
        // A reducer-load failure above already returned; a replay / kv-root-mismatch / empty-log failure here
        // maps to a clean Err (never a panic).
        let (session, report) = match seed_kv {
            Some(seed_kv) => {
                let kind = recovered.kind;
                let session = cdz_kernel::kernel::Session::recover_from_checkpoint(
                    recovered.events,
                    seed_kv,
                    &mut reducer,
                )
                .await
                .map_err(|e| format!("recover_and_build: recover_from_checkpoint failed: {e:?}"))?;
                let open_effects = session.open_effect_ids();
                (
                    session,
                    cdz_kernel::kernel::RecoveryReport { kind, open_effects },
                )
            }
            None => cdz_kernel::kernel::Session::recover_from(recovered, &mut reducer)
                .await
                .map_err(|e| format!("recover_and_build: recover_from failed: {e:?}"))?,
        };
        // The recovered session's id/genesis are already fixed by its log; build the executor set against its
        // genesis hash (its spawn-provenance for any executor that needs the session identity).
        let owner_genesis = session.genesis_hash();
        let id = crate::host::SessionId::new(owner_genesis);
        let executors = self.executors.build(&id, owner_genesis).await;
        // Re-supply the per-session collaborators EXACTLY as a fresh install (`build`) does — the durable log
        // carries none of them (`from_recovered`'s contract). A recovered session must have the SAME capability
        // grant as a fresh one (the original per-session grant isn't in the log, like the effect payload isn't;
        // `self.authz.build()` is the factory's configured grant — NOT `deny_all`, which would leave the
        // recovered session inert, unable to perform any effect its reducer re-emits, contradicting
        // `from_recovered`'s "re-supply exactly as for a fresh session" contract).
        let mut hosted = HostedSession::from_recovered(
            session,
            Box::new(reducer),
            self.authz.build(),
            executors,
        );
        // Re-attach the durable log sink + checkpoint capability so the recovered session keeps PERSISTING new
        // events (inbound folds, timeout re-drives, re-emitted effects) — a recovered durable session that ran
        // sink-less would lose everything after recovery on the next crash — and keeps its log BOUNDED via the
        // per-turn checkpoint policy. Mirrors `build`'s attach block over the SAME builder; `None` (memory
        // backend) leaves the recovered session in-memory, as it was. A fresh sink over the SAME session id
        // APPENDS to the existing durable log (the log we just recovered from), continuing it.
        if let Some(builder) = &self.log_sink {
            if let Some(sink) = builder.build(&id).await? {
                hosted = hosted.with_sink(sink);
            }
            hosted = hosted.with_checkpoint(builder.clone(), self.checkpoint_policy);
        }
        Ok((hosted, report))
    }

    /// Fetch a blob by hash from this factory's own blob store — the loop's resolver for a
    /// `control/signature` effect's target component bytes. Delegates straight to
    /// [`BlobStore::get`](cdz_kernel::blob::BlobStore::get): `Ok(Some)` present, `Ok(None)` absent (a clean
    /// miss), `Err` a backend failure (mapped to a String, never a panic).
    async fn fetch_blob(&self, hash: &Hash) -> Result<Option<bytes::Bytes>, String> {
        self.blob
            .get(hash)
            .await
            .map_err(|e| format!("blob store error fetching {hash}: {e}"))
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

    #[test]
    fn every_family_the_host_executor_suite_serves_resolves_to_a_schema_hash() {
        // PHASE-3 PRECONDITION PIN + kernel-refactor drift guard. The schema-hash effect-identity removal
        // (workstream: schema-hash-only effect identity) re-keys each host executor's family self-guard +
        // handles_family OFF content_type.family and ONTO the schema-hash, declared uniformly via
        // `cdz_kernel::ast_marshal::effect_family_schema_hash(family)` (that accessor's own doc names
        // cdz-agent-host as the consumer). That re-key REQUIRES every family this host's executors DISPATCH
        // to resolve to `Some(hash)` — a family resolving to `None` would STRAND that executor's re-key. This
        // enumerates the exact families the executor suite serves (one per leaf executor's `handles_family`)
        // and pins that all resolve, so a kernel change to the reflection table (e.g. the
        // `family_effect_schema_hash` arm-collapse `0a4d87a43`) that accidentally dropped one to `None` reds
        // HERE — a fast local signal in my own gate, before the phase-3 re-key ever hits it. Always-compiled
        // (pure const strings + a pure reflection call, no feature gates): the family→hash mapping is kernel-
        // static, independent of whether a given executor is compiled in.
        use cdz_kernel::ast_marshal::effect_family_schema_hash;
        use cdz_kernel::effect::effect_ct;
        for fam in [
            effect_ct::NOW,            // ClockExecutor
            effect_ct::HTTP,           // HttpExecutor
            effect_ct::MODEL,          // ModelExecutor
            effect_ct::EMIT,           // EmitExecutor
            effect_ct::SHELL,          // ShellExecutor (live-exec)
            effect_ct::METRIC_PUBLISH, // MetricExecutor
            effect_ct::FS_READ,        // FsExecutor (live-fs)
            effect_ct::FS_WRITE,
            effect_ct::FS_GLOB,
            effect_ct::BLOB_PUT, // BlobExecutor (live-aws-storage)
            effect_ct::BLOB_GET,
            effect_ct::LIFECYCLE_SPAWN, // LifecycleExecutor
            effect_ct::LIFECYCLE_SUSPEND,
            effect_ct::LIFECYCLE_RESUME,
            effect_ct::LIFECYCLE_TERMINATE,
            effect_ct::EFFECT_REPLY, // ReplyExecutor
            effect_ct::WS_SEND,      // WsSendExecutor (live-ws)
            effect_ct::WS_DIAL,      // WsDialExecutor (live-ws)
        ] {
            assert!(
                effect_family_schema_hash(fam).is_some(),
                "family {fam:?} — dispatched by a cdz-agent-host executor — must resolve to a schema-hash \
                 (the phase-3 re-key precondition); effect_family_schema_hash returned None, so a kernel \
                 reflection-table change dropped it and would strand that executor's re-key"
            );
        }
    }

    #[test]
    fn an_in_host_requests_schema_hash_equals_effect_family_schema_hash_of_its_family() {
        // PHASE-3 RE-KEY COMPARISON-BASIS PIN. The re-key rewrites each leaf executor's family self-guard
        // from `req.content_type.matches_family(effect_ct::X)` to a schema-hash compare
        // `req.schema_hash == effect_family_schema_hash(effect_ct::X)`. That is only BEHAVIOR-EQUIVALENT if the
        // schema-hash the kernel BAKES into an in-host request (`EffectRequest::new_with_family`, which the
        // host + kernel construct every request through) is the SAME hash the guard compares against
        // (`ast_marshal::effect_family_schema_hash`). They are computed by two separate functions today
        // (new_with_family's internal `from_family -> builtin/family memo` vs the standalone accessor), so a
        // future kernel change could silently DESYNC them — which would make a re-keyed guard reject EVERY
        // request for that family (req.schema_hash != the accessor's hash). Pin the equality for every family
        // the executor suite serves, so that desync reds HERE, in my gate, before it can strand or break the
        // re-key. (This is the companion to `every_family..resolves`: that pins Some-ness; this pins that the
        // request-carried hash and the guard-side hash are the SAME Some.)
        use cdz_kernel::ast_marshal::effect_family_schema_hash;
        use cdz_kernel::effect::{effect_ct, EffectRequest, Timeliness};
        for fam in [
            effect_ct::NOW,
            effect_ct::HTTP,
            effect_ct::MODEL,
            effect_ct::EMIT,
            effect_ct::SHELL,
            effect_ct::METRIC_PUBLISH,
            effect_ct::FS_READ,
            effect_ct::FS_WRITE,
            effect_ct::FS_GLOB,
            effect_ct::BLOB_PUT,
            effect_ct::BLOB_GET,
            effect_ct::LIFECYCLE_SPAWN,
            effect_ct::LIFECYCLE_SUSPEND,
            effect_ct::LIFECYCLE_RESUME,
            effect_ct::LIFECYCLE_TERMINATE,
            effect_ct::EFFECT_REPLY,
            effect_ct::WS_SEND,
            effect_ct::WS_DIAL,
        ] {
            let req = EffectRequest::new_with_family(fam, "t", None, Timeliness::Interactive);
            assert_eq!(
                req.schema_hash,
                effect_family_schema_hash(fam),
                "family {fam:?}: the schema-hash baked into an in-host EffectRequest (new_with_family) must \
                 equal effect_family_schema_hash(family) — the phase-3 re-keyed guard compares the former \
                 against the latter, so a desync would make the guard reject every request for this family"
            );
        }
    }

    #[test]
    fn non_executor_families_the_phase3_re_key_depends_on_also_resolve_and_bake_consistently() {
        // PHASE-3 RE-KEY COVERAGE-GAP GUARD (companion to the two executor-suite pins above). The re-key does
        // NOT only touch leaf-executor self-guards — it also re-keys two host surfaces that dispatch on
        // schema-hash for families NO cdz-agent-host leaf executor serves:
        //   - the host loop's CONTROL-effect match — it keys control/summary (== effect_family_schema_hash(
        //     SUMMARY)) and control/signature (!= effect_family_schema_hash(SIGNATURE)) independently;
        //   - the genesis/kv reducer E2E schema_hash asserts (host.rs real_pure/kv_reducer_*), which pin an
        //     emitted `timer` effect's baked schema_hash against effect_family_schema_hash(TIMER).
        // TIMER (kernel-INJECTED — timer.arm has no leaf executor) and SUMMARY/SIGNATURE (host-loop-handled
        // control families) are therefore ABSENT from `every_family_the_host_executor_suite_serves_resolves`
        // + `an_in_host_requests_schema_hash_equals_...`, which enumerate only the executor-suite families.
        // Yet the re-key compares a baked schema_hash against `effect_family_schema_hash(fam)` for exactly
        // these. A kernel reflection-table change that dropped one to None — or desynced its baked hash —
        // would break the control match / E2E asserts. And the E2E asserts are ENV-GATED (they SKIP on a
        // bare `cargo test` when the reducer components + CDZ_STORE are unset), so the `timer` break would
        // slip PAST the local hermetic gate and red only in a non-vacuous CI run. Pin resolve + bake-
        // consistency HERE so the whole re-key-depended-upon family set reds hermetically in my own gate.
        use cdz_kernel::ast_marshal::effect_family_schema_hash;
        use cdz_kernel::effect::{effect_ct, EffectRequest, Timeliness};
        for fam in [effect_ct::TIMER, effect_ct::SUMMARY, effect_ct::SIGNATURE] {
            assert!(
                effect_family_schema_hash(fam).is_some(),
                "family {fam:?} — depended on by the phase-3 re-key (host control match / env-gated E2E \
                 schema_hash assert) but served by NO leaf executor — must resolve to a schema-hash; None \
                 would break the re-key and (for timer) slip past the hermetic gate since its E2E is env-gated"
            );
            let req = EffectRequest::new_with_family(fam, "t", None, Timeliness::Interactive);
            assert_eq!(
                req.schema_hash,
                effect_family_schema_hash(fam),
                "family {fam:?}: the in-host-baked schema_hash (new_with_family) must equal \
                 effect_family_schema_hash(family) — the re-keyed control match + E2E asserts compare the \
                 former against the latter, so a desync would misfire them"
            );
        }
        // The control match distinguishes control/summary (matched with ==) from control/signature (matched
        // with !=): they MUST hash DISTINCTLY, else the two control paths would cross-fire on each other. The
        // executor-suite pins never needed a distinctness check; the control match does.
        assert_ne!(
            effect_family_schema_hash(effect_ct::SUMMARY),
            effect_family_schema_hash(effect_ct::SIGNATURE),
            "control/summary and control/signature must hash distinctly — the host loop's control match keys \
             on each independently (== SUMMARY, != SIGNATURE); a collision would cross-fire the control paths"
        );
    }

    #[tokio::test]
    async fn install_of_an_absent_reducer_hash_is_a_clean_error() {
        // The blob store is empty → get returns None → build errors cleanly (no panic), and apply_admin
        // surfaces it as an AdminResponse::Error with the registry untouched.
        let mut host = AgentHost::new();
        let mut factory =
            ComponentSessionFactory::new(MemBlobStore::new(), hermetic_executors, deny_all_authz);
        let spec = InstallSpec {
            id: SessionId::new(Hash::of(b"ghost")),
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
        let hash = cdz_kernel::hash::Hash::of(&garbage);
        blob.put(hash, bytes::Bytes::from(garbage.clone()))
            .await
            .unwrap();
        let mut factory = ComponentSessionFactory::new(blob, hermetic_executors, deny_all_authz);
        let spec = InstallSpec {
            id: SessionId::new(Hash::of(b"bad")),
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
    async fn recover_and_build_reads_the_reducer_hash_from_the_recovered_genesis_and_errors_cleanly_if_absent(
    ) {
        // I4b: recover_and_build reads the reducer hash from the recovered log's OWN genesis event (events[0],
        // not an InstallSpec) + reloads the reducer from the blob store. Here the reducer bytes are ABSENT → a
        // clean "no reducer component" error keyed on THAT genesis hash (proving it read the hash from the
        // recovered log). Build a genesis Session over a known reducer hash to get a valid genesis log, then
        // hand that log's Recovered to recover_and_build against an empty blob store.
        use cdz_kernel::kernel::Session;
        use cdz_kernel::log_store::{Recovered, RecoveryKind};

        let reducer_hash = Hash::of(b"recovered-reducer-not-in-store");
        let original = Session::genesis(reducer_hash, Hash::of(b"nonce"));
        // A never-delivered session's durable log is exactly its genesis event (log-decouple I5: read the
        // genesis off the derived `genesis_ref`, not the resident Vec).
        let recovered = Recovered {
            events: vec![original.genesis_ref().clone()],
            kind: RecoveryKind::Clean,
            good_prefix_len: 0,
        };

        let mut factory =
            ComponentSessionFactory::new(MemBlobStore::new(), hermetic_executors, deny_all_authz);
        let err = match factory.recover_and_build(recovered).await {
            Ok(_) => {
                panic!("recover_and_build must fail when the recovered reducer bytes are absent")
            }
            Err(e) => e,
        };
        assert!(
            err.contains("no reducer component in the blob store for recovered hash")
                && err.contains(&reducer_hash.to_hex()),
            "the error names the reducer hash read FROM the recovered genesis: {err}"
        );
    }

    /// A `[Genesis, Checkpoint@N]` checkpoint-rooted log for recovery tests — the shape
    /// [`FileLogSinkBuilder::checkpoint`] leaves on disk. The checkpoint frame comes from the kernel
    /// (`checkpoint_frame`), so its descriptor's `kv_root` is authoritative.
    fn checkpoint_rooted_recovered(reducer_hash: Hash) -> cdz_kernel::log_store::Recovered {
        use cdz_kernel::kernel::Session;
        use cdz_kernel::log_store::{Recovered, RecoveryKind};
        let session = Session::genesis(reducer_hash, Hash::of(b"cp-nonce"));
        Recovered {
            events: vec![session.genesis_ref().clone(), session.checkpoint_frame()],
            kind: RecoveryKind::Clean,
            good_prefix_len: 0,
        }
    }

    #[tokio::test]
    async fn recover_and_build_of_a_checkpoint_rooted_log_errors_when_no_snapshot_store_is_configured(
    ) {
        // GAP-4 D3-2b: a checkpoint-rooted log ([Genesis, Checkpoint@N, tail]) cannot be recovered by full
        // replay (the subsumed prefix is gone; a Checkpoint frame is a seed, not a foldable event). If the
        // factory has NO log sink, there is no checkpoint store to load the seed-KV snapshot from → a clean
        // Err naming the missing kv_root, resolved BEFORE any reducer fetch (a factory with an empty blob
        // store must still surface the snapshot-store gap, not a spurious reducer-absent error).
        let reducer_hash = Hash::of(b"cp-reducer");
        let recovered = checkpoint_rooted_recovered(reducer_hash);
        let mut factory =
            ComponentSessionFactory::new(MemBlobStore::new(), hermetic_executors, deny_all_authz);
        let err = match factory.recover_and_build(recovered).await {
            Ok(_) => panic!("a checkpoint-rooted log with no snapshot store must not recover"),
            Err(e) => e,
        };
        assert!(
            err.contains("no log sink is configured to load the seed KV snapshot"),
            "the error names the missing checkpoint snapshot store, not a reducer miss: {err}"
        );
    }

    #[tokio::test]
    async fn recover_and_build_of_a_checkpoint_rooted_log_errors_when_the_snapshot_is_absent() {
        // GAP-4 D3-2b: the snapshot store is configured but the descriptor's kv_root snapshot isn't present
        // (declines with None) → a clean Err, again resolved before the reducer fetch. A builder whose
        // load_checkpoint_snapshot returns None stands in for a checkpoint store missing the named snapshot.
        struct NoSnapshotBuilder;
        #[async_trait::async_trait(?Send)]
        impl LogSinkBuilder for NoSnapshotBuilder {
            async fn build(
                &self,
                _id: &SessionId,
            ) -> Result<Option<Box<dyn cdz_kernel::log_store::LogSink>>, String> {
                Ok(None)
            }
            // load_checkpoint_snapshot uses the trait default (Ok(None)) — the "snapshot absent" case.
        }
        let reducer_hash = Hash::of(b"cp-reducer");
        let recovered = checkpoint_rooted_recovered(reducer_hash);
        let mut factory =
            ComponentSessionFactory::new(MemBlobStore::new(), hermetic_executors, deny_all_authz)
                .with_log_sink(Box::new(NoSnapshotBuilder));
        let err = match factory.recover_and_build(recovered).await {
            Ok(_) => panic!("a checkpoint-rooted log with an absent snapshot must not recover"),
            Err(e) => e,
        };
        assert!(
            err.contains("no matching KV snapshot"),
            "the error names the absent checkpoint snapshot: {err}"
        );
    }

    #[tokio::test]
    async fn recover_and_build_of_a_checkpoint_rooted_log_loads_the_snapshot_then_reaches_the_reducer(
    ) {
        // GAP-4 D3-2b: when the snapshot store DOES have the seed KV, resolution succeeds and recovery
        // threads on to the reducer fetch — proving the checkpoint branch loads the snapshot and proceeds
        // (rather than short-circuiting). Here the blob store has no reducer, so the NEXT step (reducer load)
        // is what fails, keyed on the recovered genesis hash — the snapshot resolution itself did NOT error.
        struct SnapshotPresentBuilder;
        #[async_trait::async_trait(?Send)]
        impl LogSinkBuilder for SnapshotPresentBuilder {
            async fn build(
                &self,
                _id: &SessionId,
            ) -> Result<Option<Box<dyn cdz_kernel::log_store::LogSink>>, String> {
                Ok(None)
            }
            async fn load_checkpoint_snapshot(
                &self,
                _kv_root: Hash,
            ) -> Result<Option<cdz_kernel::kv::Kv>, String> {
                // A snapshot is present (its exact content is validated against kv_root only once recovery
                // reaches recover_from_checkpoint — here recovery errors earlier at the reducer fetch).
                Ok(Some(cdz_kernel::kv::Kv::new()))
            }
        }
        let reducer_hash = Hash::of(b"cp-reducer-not-in-store");
        let recovered = checkpoint_rooted_recovered(reducer_hash);
        let mut factory =
            ComponentSessionFactory::new(MemBlobStore::new(), hermetic_executors, deny_all_authz)
                .with_log_sink(Box::new(SnapshotPresentBuilder));
        let err = match factory.recover_and_build(recovered).await {
            Ok(_) => panic!("the reducer bytes are absent, so recovery must still fail after loading the snapshot"),
            Err(e) => e,
        };
        assert!(
            err.contains("no reducer component in the blob store for recovered hash")
                && err.contains(&reducer_hash.to_hex()),
            "snapshot resolution succeeded and recovery advanced to the reducer fetch: {err}"
        );
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
        let hash = cdz_kernel::hash::Hash::of(&bytes);
        blob.put(hash, bytes::Bytes::from(bytes.clone()))
            .await
            .unwrap();
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
                    id: SessionId::new(Hash::of(b"real")),
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
                id: SessionId::new(Hash::of(b"real"))
            },
            "a real reducer component installs"
        );

        // Drive the installed session: deliver a `message` inbound → the reducer's fold runs → `count`=1.
        use cdz_kernel::effect::Payload;
        use cdz_kernel::event::{ContentType, EventBody};
        let outcome = host
            .deliver(
                &SessionId::new(Hash::of(b"real")),
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
            host.get(&SessionId::new(Hash::of(b"real")))
                .unwrap()
                .session()
                .kv()
                .get(b"count")
                .as_deref(),
            Some(&[1u8][..]),
            "the reducer's fold bumped its count counter — the agent ran"
        );
    }

    #[tokio::test]
    async fn file_log_sink_builder_opens_a_per_session_log() {
        // The [log]=file sink builder: build(id) opens a LogStore at <root>/<id-hex>.log and returns Some. A
        // fresh path yields a usable sink + the file exists after open. Unique proven-fresh root (no fixed
        // temp path — #1991 review).
        let root = crate::testutil::unique_temp_dir("filelog");
        let builder = FileLogSinkBuilder::new(&root);
        let id = SessionId::new(Hash::of(b"sess-1"));
        let sink = builder
            .build(&id)
            .await
            .expect("build ok")
            .expect("file backend yields a sink");
        // The sink is a real LogStore; dropping it is fine — the point is the per-session file was created.
        drop(sink);
        assert!(
            root.join(format!("{}.log", id.to_hex())).exists(),
            "per-session log file created at <root>/<id-hex>.log"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn file_log_sink_builder_keeps_the_log_under_root_as_a_single_component() {
        // The session id is a genesis-hash HEX (`[0-9a-f]{64}`) — a single safe filename component by
        // construction (no path separators), so the log file can NEVER escape the root. This pins that
        // path-safety invariant (which replaced the old id-string sanitization — a Hash id can't hold a '/').
        let root = crate::testutil::unique_temp_dir("filelog-safe");
        let builder = FileLogSinkBuilder::new(&root);
        let id = SessionId::new(Hash::of(b"evil/../escape"));
        let _sink = builder.build(&id).await.expect("build ok").expect("sink");
        // Exactly one file, directly under root, named <id-hex>.log — no traversal, no nested dirs.
        let entries: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries, vec![format!("{}.log", id.to_hex())], "{entries:?}");
        assert!(
            id.to_hex().chars().all(|c| c.is_ascii_hexdigit()),
            "a session id renders as pure hex — no path separator can appear"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn file_log_sink_builder_recovers_an_appended_log_round_trip() {
        // Boot-recovery read path (I4b): append events through the SAME builder's sink, then recover(id) reads
        // them back in order via LogStore::recover. build (append) + recover (read) must agree on the path, so
        // this proves the round-trip through the LogSinkBuilder trait the daemon uses.
        use cdz_kernel::event::{Event, EventBody};
        use cdz_kernel::hash::Hash;
        use cdz_kernel::log_store::RecoveryKind;

        let root = crate::testutil::unique_temp_dir("filelog-recover");
        let builder = FileLogSinkBuilder::new(&root);
        let id = SessionId::new(Hash::of(b"sess-recover"));
        let genesis = Event {
            seq: 0,
            cause: None,
            body: EventBody::Genesis {
                reducer: Hash::of(b"reducer"),
                spawn_nonce: Hash::of(b"nonce"),
                parent: None,
            },
        };
        {
            let mut sink = builder
                .build(&id)
                .await
                .expect("build ok")
                .expect("file backend yields a sink");
            sink.append(&genesis).await.expect("append genesis");
        } // drop the sink so the file is fully flushed/closed before recovery reads it.

        let recovered = builder
            .recover(&id)
            .await
            .expect("recover ok")
            .expect("an existing file log recovers to Some");
        assert_eq!(recovered.kind, RecoveryKind::Clean, "a whole log is Clean");
        assert_eq!(recovered.events.len(), 1, "the one appended event recovers");
        assert!(
            matches!(recovered.events[0].body, EventBody::Genesis { .. }),
            "the recovered event is the genesis we appended"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn offload_source_dir_materializes_a_working_cas() {
        // GAP-4 D1: OffloadSource::Dir materializes a raw DiskBlobStore that round-trips a put/get — the handle
        // each per-session sink opens over the shared [blob] dir. (The end-to-end offload path is exercised by
        // the round-trip test below; this pins the source->store materialization in isolation.)
        let dir = crate::testutil::unique_temp_dir("offload-src-dir");
        let mut blob = OffloadSource::Dir(dir.clone())
            .materialize()
            .expect("Dir source materializes a blob store");
        let h = cdz_kernel::hash::Hash::of(b"body");
        blob.put(h, bytes::Bytes::from_static(b"body"))
            .await
            .expect("put");
        assert_eq!(
            blob.get(&h).await.expect("get").as_deref(),
            Some(&b"body"[..]),
            "the materialized Dir CAS round-trips a blob"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(feature = "live-aws-storage")]
    #[test]
    fn offload_source_s3_materializes_a_blob_store() {
        // GAP-4 D1 S3 offload (the prod Dynamo+S3 / File+S3 pairing): OffloadSource::S3 materializes an
        // S3BlobStore from a shared SdkConfig — a cheap client clone (from_conf), no live call to CONSTRUCT.
        // (A real put/get would hit S3, out of scope for a hermetic test; this pins the source wiring.)
        let config = aws_config::SdkConfig::builder()
            .behavior_version(aws_config::BehaviorVersion::latest())
            .build();
        let source = OffloadSource::S3 {
            config,
            bucket: "cdz-blobs".into(),
            prefix: "logs/".into(),
        };
        assert!(
            source.materialize().is_ok(),
            "S3 offload source materializes a blob store handle from a shared SdkConfig"
        );
    }

    #[tokio::test]
    async fn file_log_sink_builder_offload_round_trips_byte_identical() {
        // GAP-4 D1 slice-1 PIN: with body-offload configured, an OVER-threshold event body is offloaded to the
        // content-addressed blob store (a tiny (blob-ptr) frame replaces it in the log), and recover()
        // REHYDRATES it byte-identical — same event as appended. A SUB-threshold body stays inline. Proves the
        // FileLogSinkBuilder with_offload + recover_rehydrated wiring end-to-end through the LogSinkBuilder
        // trait the daemon uses (the offload store is a raw DiskBlobStore over the same [blob] root).
        use cdz_kernel::effect::Payload;
        use cdz_kernel::event::{ContentType, Event, EventBody};
        use cdz_kernel::hash::Hash;
        use cdz_kernel::log_store::RecoveryKind;

        let root = crate::testutil::unique_temp_dir("filelog-offload");
        let blob_dir = crate::testutil::unique_temp_dir("filelog-offload-cas");
        // Threshold 64 bytes: the big Inbound body offloads, the small one stays inline.
        let builder =
            FileLogSinkBuilder::new(&root).with_offload(OffloadSource::Dir(blob_dir.clone()), 64);
        let id = SessionId::new(Hash::of(b"sess-offload"));

        let genesis = Event {
            seq: 0,
            cause: None,
            body: EventBody::Genesis {
                reducer: Hash::of(b"reducer"),
                spawn_nonce: Hash::of(b"nonce"),
                parent: None,
            },
        };
        // seq 1: a BIG body (256 bytes > 64 threshold) → offloaded to the CAS.
        let big_payload = vec![0xabu8; 256];
        let big = Event {
            seq: 1,
            cause: None,
            body: EventBody::Inbound {
                content_type: ContentType {
                    family: "message".into(),
                    version: 1,
                },
                payload: Payload::Inline(big_payload.clone().into()),
            },
        };
        // seq 2: a SMALL body (< 64) → stays inline.
        let small = Event {
            seq: 2,
            cause: None,
            body: EventBody::Inbound {
                content_type: ContentType {
                    family: "message".into(),
                    version: 1,
                },
                payload: Payload::Inline(b"small".to_vec().into()),
            },
        };
        {
            let mut sink = builder
                .build(&id)
                .await
                .expect("build ok")
                .expect("file backend yields a sink");
            sink.append(&genesis).await.expect("append genesis");
            sink.append(&big).await.expect("append big");
            sink.append(&small).await.expect("append small");
        } // drop → flush/close before recovery reads it.

        // The over-threshold body was actually OFFLOADED: the CAS dir holds at least one blob (non-vacuous —
        // if nothing offloaded, the round-trip below would pass trivially with everything inline).
        let cas_entries = std::fs::read_dir(&blob_dir).map(|d| d.count()).unwrap_or(0);
        assert!(
            cas_entries >= 1,
            "the over-threshold body was offloaded to the CAS (got {cas_entries} blob(s))"
        );

        // Recovery REHYDRATES: all three events come back byte-identical, including the offloaded big body.
        let recovered = builder
            .recover(&id)
            .await
            .expect("recover ok")
            .expect("an existing offloaded log recovers to Some");
        assert_eq!(
            recovered.kind,
            RecoveryKind::Clean,
            "a whole offloaded+rehydrated log is Clean"
        );
        assert_eq!(recovered.events, vec![genesis, big, small],
            "the offloaded log recovers BYTE-IDENTICAL after rehydrate (the big body derefed from the CAS)");

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&blob_dir);
    }

    #[tokio::test]
    async fn file_log_sink_builder_recover_of_a_missing_session_is_none() {
        // A session that was never persisted → recover is Ok(None) (nothing to resurrect), NOT an error — the
        // boot loop skips it cleanly.
        let root = crate::testutil::unique_temp_dir("filelog-recover-missing");
        let builder = FileLogSinkBuilder::new(&root);
        let got = builder
            .recover(&SessionId::new(Hash::of(b"never-existed")))
            .await
            .expect("recover of a missing log is Ok, not Err");
        assert!(got.is_none(), "a missing session log recovers as None");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn log_sink_builder_recover_defaults_to_none() {
        // The trait default (a builder that doesn't override recover) declines boot-recovery cleanly — the
        // memory/no-durable-log backend has nothing to recover.
        struct NoRecoverBuilder;
        #[async_trait::async_trait(?Send)]
        impl LogSinkBuilder for NoRecoverBuilder {
            async fn build(
                &self,
                _id: &SessionId,
            ) -> Result<Option<Box<dyn cdz_kernel::log_store::LogSink>>, String> {
                Ok(None)
            }
        }
        let got = NoRecoverBuilder
            .recover(&SessionId::new(Hash::of(b"any")))
            .await
            .expect("default recover is Ok");
        assert!(got.is_none(), "the trait default declines with None");
    }

    #[tokio::test]
    async fn file_log_sink_checkpoint_rewrites_to_genesis_plus_checkpoint_and_persists_the_snapshot(
    ) {
        // GAP-4 D3-2a: FileLogSinkBuilder::checkpoint rewrites a session's durable file log to
        // [Genesis, Checkpoint@N] (dropping the subsumed 1..=N prefix) and persists the KV snapshot to the
        // offload store, content-addressed by the descriptor's kv_root. Crash-safe (temp + atomic rename).
        use crate::host::HostedSession;
        use cdz_kernel::event::{ContentType, Event, EventBody};
        use cdz_kernel::executor::CompositeExecutor;
        use cdz_kernel::kv::Kv;
        use cdz_kernel::log_store::LogStore;
        use cdz_kernel::reducer::{FoldOutput, Reducer};

        // A no-effect reducer: records each message payload in KV, dispatches nothing — so the session
        // advances + KV fills WITHOUT open obligations (a clean checkpoint).
        struct KvPutReducer;
        #[async_trait::async_trait(?Send)]
        impl Reducer for KvPutReducer {
            async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
                if let EventBody::Inbound {
                    payload: cdz_kernel::effect::Payload::Inline(b),
                    ..
                } = &event.body
                {
                    kv.put(b"last".to_vec(), b.to_vec());
                }
                FoldOutput::none()
            }
        }
        fn message(v: &[u8]) -> EventBody {
            EventBody::Inbound {
                content_type: ContentType {
                    family: "message".into(),
                    version: 1,
                },
                payload: cdz_kernel::effect::Payload::Inline(v.to_vec().into()),
            }
        }

        let root = crate::testutil::unique_temp_dir("d3-2a-cp");
        let blob_dir = crate::testutil::unique_temp_dir("d3-2a-cas");
        // High offload threshold: the log FRAMES stay inline; the offload store here backs the checkpoint
        // SNAPSHOT (D3-1), which checkpoint requires.
        let builder = FileLogSinkBuilder::new(&root)
            .with_offload(OffloadSource::Dir(blob_dir.clone()), 1 << 20);

        // Build a session, seed its genesis into a recording sink (with_sink attaches post-genesis, so the
        // seeded buffer is the COMPLETE log), deliver two messages (tip -> seq 2, KV "last"=v2).
        let base = HostedSession::genesis(
            Hash::of(b"d3-2a-reducer"),
            Box::new(KvPutReducer),
            deny_all_authz(),
            CompositeExecutor::new(),
        );
        let id = SessionId::new(base.genesis_hash());
        let genesis = base.session().genesis_ref().clone();
        let (sink, captured) = crate::testutil::log_capture::recording_sink_seeded(genesis);
        let mut hosted = base.with_sink(sink);
        hosted
            .deliver(message(b"v1"), None)
            .await
            .expect("deliver v1");
        hosted
            .deliver(message(b"v2"), None)
            .await
            .expect("deliver v2");

        // Materialize the genesis-seeded durable log into the real per-session file the builder checkpoints.
        let path = builder.log_path(&id);
        {
            let mut store = LogStore::open(&path).expect("open durable log");
            for e in captured.borrow().iter() {
                store.append(e).expect("seed durable frame");
            }
        }
        let before = LogStore::recover(&path).expect("recover pre-checkpoint");
        assert!(
            before.events.len() >= 3,
            "pre-checkpoint log has genesis + a delivered prefix to subsume, got {}",
            before.events.len()
        );
        let kv_root = hosted.session().kv().root_hash();

        // CHECKPOINT.
        builder
            .checkpoint(&id, hosted.session())
            .await
            .expect("checkpoint succeeds");

        // The log is now exactly [Genesis, Checkpoint@N] — the subsumed prefix is gone.
        let after = LogStore::recover(&path).expect("recover post-checkpoint");
        assert_eq!(
            after.events.len(),
            2,
            "post-checkpoint log is [Genesis, Checkpoint], got {}",
            after.events.len()
        );
        assert!(
            matches!(after.events[0].body, EventBody::Genesis { .. }),
            "events[0] stays the genesis"
        );
        let desc = match &after.events[1].body {
            EventBody::Checkpoint(d) => d.clone(),
            other => panic!("events[1] must be the Checkpoint frame, got {other:?}"),
        };
        assert_eq!(
            desc.kv_root, kv_root,
            "the checkpoint frame's kv_root is the session's current KV root"
        );
        assert!(
            !after
                .events
                .iter()
                .any(|e| matches!(e.body, EventBody::Inbound { .. })),
            "the subsumed Inbound prefix was pruned"
        );

        // The KV snapshot was persisted to the offload store under kv_root and reloads to the live KV.
        let blob = OffloadSource::Dir(blob_dir.clone())
            .materialize()
            .expect("materialize snapshot store");
        let snap = crate::checkpoint::load_kv_snapshot(blob.as_ref(), kv_root)
            .await
            .expect("load snapshot ok")
            .expect("the checkpoint persisted its KV snapshot");
        assert_eq!(
            snap.root_hash(),
            kv_root,
            "reloaded snapshot matches the descriptor kv_root"
        );
        assert_eq!(
            snap.get(b"last").as_deref(),
            Some(&b"v2"[..]),
            "the snapshot carries the session's committed KV content"
        );

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&blob_dir);
    }

    #[tokio::test]
    async fn load_checkpoint_snapshot_resolves_the_offload_store_and_declines_when_absent_or_unconfigured(
    ) {
        // GAP-4 D3-2b (recovery-loader): FileLogSinkBuilder::load_checkpoint_snapshot reads a checkpoint's KV
        // snapshot back from the SAME offload store checkpoint writes it to, keyed by the descriptor's kv_root
        // — the seed_kv recover_from_checkpoint needs. Absent snapshot -> None (fall back to replay); no
        // offload store -> None (that log never checkpoints).
        use cdz_kernel::kv::Kv;
        let root = crate::testutil::unique_temp_dir("d3-2b-cp");
        let blob_dir = crate::testutil::unique_temp_dir("d3-2b-cas");
        let builder = FileLogSinkBuilder::new(&root)
            .with_offload(OffloadSource::Dir(blob_dir.clone()), 1 << 20);

        // Persist a snapshot to the builder's offload store (the SAME store checkpoint writes to), then load
        // it back through the trait method by its kv_root.
        let mut kv = Kv::new();
        kv.put(b"k".to_vec(), b"v".to_vec());
        let kv_root = {
            let mut blob = OffloadSource::Dir(blob_dir.clone())
                .materialize()
                .expect("materialize");
            crate::checkpoint::persist_kv_snapshot(blob.as_mut(), &kv)
                .await
                .expect("persist")
        };
        let loaded = builder
            .load_checkpoint_snapshot(kv_root)
            .await
            .expect("load ok")
            .expect("the persisted snapshot is present");
        assert_eq!(
            loaded.root_hash(),
            kv_root,
            "loaded snapshot matches the kv_root"
        );
        assert_eq!(loaded.get(b"k").as_deref(), Some(&b"v"[..]));

        // A kv_root never persisted -> None (the caller falls back to a full-log replay).
        let absent = {
            let mut e = Kv::new();
            e.put(b"other".to_vec(), b"x".to_vec());
            e.root_hash()
        };
        assert!(
            builder
                .load_checkpoint_snapshot(absent)
                .await
                .expect("get ok")
                .is_none(),
            "an unpersisted kv_root loads as None"
        );

        // A builder with NO offload store configured -> None (that log never checkpoints, so no snapshot).
        let no_offload = FileLogSinkBuilder::new(&root);
        assert!(
            no_offload
                .load_checkpoint_snapshot(kv_root)
                .await
                .expect("no-offload load ok")
                .is_none(),
            "no offload store configured -> None"
        );

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&blob_dir);
    }

    /// A stub executor that returns a SCRIPTED sequence of outcomes (one per `perform`), for exercising the
    /// metering decorator's classification without a real transport.
    struct ScriptedExecutor {
        outcomes: std::cell::RefCell<std::collections::VecDeque<EffectOutcome>>,
    }
    #[async_trait::async_trait(?Send)]
    impl Executor for ScriptedExecutor {
        async fn perform(
            &mut self,
            _id: EffectId,
            _req: &EffectRequest,
            _key: Hash,
        ) -> EffectOutcome {
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

        let req = EffectRequest::new(EffectKind::Model, "m", None, Timeliness::Interactive);
        // Each outcome passes THROUGH unchanged (the decorator is transparent) — the wrap classifies +
        // records each into the registry along the way (registry Counters are drain-on-report with no value
        // getter, so we assert pass-through + a reportable registry, not per-counter values).
        assert!(matches!(
            metered.perform(EffectId(0), &req, Hash::of(b"k")).await,
            EffectOutcome::Ok(_)
        ));
        assert!(matches!(
            metered.perform(EffectId(0), &req, Hash::of(b"k")).await,
            EffectOutcome::Err { .. }
        ));
        metered.perform(EffectId(0), &req, Hash::of(b"k")).await;
        metered.perform(EffectId(0), &req, Hash::of(b"k")).await;
        assert!(matches!(
            metered.perform(EffectId(0), &req, Hash::of(b"k")).await,
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
        use cdz_kernel::effect::{Payload, Timeliness};
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

        // Route one effect to each family (CompositeExecutor dispatches a register-by-string family by its
        // `string_routed_family` carrier). `new_with_family` populates the carrier for a register-by-string
        // extension family (is_registered_effect_family), so these route without a hand-patched field.
        let req_a = EffectRequest::new_with_family("fam-a", "t", None, Timeliness::Interactive);
        let req_b = EffectRequest::new_with_family("fam-b", "t", None, Timeliness::Interactive);
        composite.perform(EffectId(0), &req_a, Hash::of(b"a")).await;
        composite.perform(EffectId(0), &req_b, Hash::of(b"b")).await;

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
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
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
        let policy_hash = cdz_kernel::hash::Hash::of(&bytes);
        blob.put(policy_hash, bytes::Bytes::from(bytes.clone()))
            .await
            .unwrap();
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
            &SessionId::new(Hash::of(b"genesis-test")),
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
            session
                .session()
                .kv()
                .get(genesis_ct::KV_ROOT_IDENTITY)
                .as_deref(),
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
        assert_eq!(
            kv.get(genesis_ct::KV_ROOT_IDENTITY).as_deref(),
            Some(&b"root"[..])
        );
        assert_eq!(
            kv.get(genesis_ct::KV_AUTHORIZER_HASH).as_deref(),
            Some(missing.as_bytes().as_slice()),
            "the authorizer-hash was seeded before the install step failed (half-booted)"
        );
    }

    #[tokio::test]
    async fn recover_and_build_is_reachable_through_the_boxed_factory_trait() {
        use crate::admin::SessionFactory;
        use cdz_kernel::log_store::{Recovered, RecoveryKind};

        // The boot-recovery loop holds the factory as `Box<dyn SessionFactory>`, so recover_and_build MUST be a
        // trait method (not just inherent on the concrete type). Drive it through the trait object: a recovered
        // log whose genesis references a reducer hash ABSENT from the (empty) blob store hits the "no reducer
        // component" error — proving dispatch reached the concrete override + read the reducer hash off the
        // recovered genesis (all hermetic, no real wasm component needed).
        let original = genesis_session();
        // genesis_session() mints with this reducer hash; recover_and_build reads it back off the recovered
        // genesis event, so the error names it.
        let reducer_hash = Hash::of(b"genesis-reducer-v1");
        let recovered = Recovered {
            events: vec![original.session().genesis_ref().clone()],
            kind: RecoveryKind::Clean,
            good_prefix_len: 0,
        };

        let mut factory: Box<dyn SessionFactory> = Box::new(ComponentSessionFactory::new(
            MemBlobStore::new(),
            hermetic_executors,
            deny_all_authz,
        ));
        let err = factory.recover_and_build(recovered).await.err().expect(
            "an empty blob store has no reducer component to rebuild the recovered session",
        );
        assert!(
            err.contains("no reducer component in the blob store for recovered hash")
                && err.contains(&reducer_hash.to_string()),
            "{err}"
        );
    }

    #[tokio::test]
    async fn fetch_blob_returns_a_put_blob_and_none_for_absent() {
        use crate::admin::SessionFactory;
        // The loop's signature-query target resolver: a blob put into the factory's store is fetched back by
        // hash; an absent hash is a clean Ok(None) (a miss the loop settles as the Err arm), not an error.
        let mut blob = MemBlobStore::new();
        let bytes = b"a target component's bytes".to_vec();
        let hash = cdz_kernel::hash::Hash::of(&bytes);
        blob.put(hash, bytes::Bytes::from(bytes.clone()))
            .await
            .unwrap();
        let factory = ComponentSessionFactory::new(blob, hermetic_executors, deny_all_authz);
        assert_eq!(
            factory.fetch_blob(&hash).await.unwrap().as_deref(),
            Some(&bytes[..]),
            "a put blob is fetched back by hash"
        );
        assert_eq!(
            factory
                .fetch_blob(&Hash::of(b"never-stored"))
                .await
                .unwrap(),
            None,
            "an absent hash is a clean miss (Ok(None)), not an error"
        );
    }

    #[tokio::test]
    async fn fetch_blob_default_is_none() {
        use crate::admin::{InstallSpec, SessionFactory};
        // A factory with no blob store (a stub) resolves every hash as absent via the trait default, so the
        // loop settles the signature query's Err arm cleanly rather than the trait forcing a blob store.
        struct NoBlobFactory;
        #[async_trait::async_trait(?Send)]
        impl SessionFactory for NoBlobFactory {
            async fn build(&mut self, _spec: &InstallSpec) -> Result<HostedSession, String> {
                Err("stub".into())
            }
        }
        assert_eq!(
            NoBlobFactory.fetch_blob(&Hash::of(b"x")).await.unwrap(),
            None,
            "the trait default resolves every hash as absent"
        );
    }

    #[tokio::test]
    async fn recover_and_build_default_is_an_unsupported_error() {
        use crate::admin::{InstallSpec, SessionFactory};
        use cdz_kernel::log_store::{Recovered, RecoveryKind};

        // A factory that does NOT override recover_and_build (only build) declines boot-recovery cleanly via
        // the trait default — never a panic, so a recovery loop against such a factory surfaces the decline.
        struct InstallOnlyFactory;
        #[async_trait::async_trait(?Send)]
        impl SessionFactory for InstallOnlyFactory {
            async fn build(&mut self, _spec: &InstallSpec) -> Result<HostedSession, String> {
                Ok(genesis_session())
            }
        }

        let original = genesis_session();
        let recovered = Recovered {
            events: vec![original.session().genesis_ref().clone()],
            kind: RecoveryKind::Clean,
            good_prefix_len: 0,
        };
        let mut factory: Box<dyn SessionFactory> = Box::new(InstallOnlyFactory);
        let err = factory
            .recover_and_build(recovered)
            .await
            .err()
            .expect("the trait default declines boot-recovery");
        assert!(err.contains("does not support boot-recovery"), "{err}");
    }
}
