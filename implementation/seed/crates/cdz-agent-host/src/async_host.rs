//! The async multi-session host loop — cooperatively interleaves many agent sessions on ONE task.
//!
//! [`AgentHost`] (sync) holds the registry + drives one session per call. This wraps it in a
//! **single-threaded async event loop**: sessions feed a shared inbound channel, and one loop `select!`s
//! over that channel + a timer (the earliest armed-timer deadline across all sessions) + a shutdown
//! signal, driving the addressed session IN-PLACE when its input is ready. Many agents run concurrently
//! (interleaved), each still on the single-threaded kernel loop that replay-determinism needs (§15b).
//!
//! **Why single-threaded (not a task per session):** the kernel's `Reducer`/`Authorize`/`Executor` are
//! not `: Send`, and a `ComponentReducer` holds wasmtime types — so a session can't move into its own
//! tokio task. That's by design, confirmed with v-agent-harness: a session's folds MUST be sequential
//! (replay-determinism), and agents are I/O-bound on effects, so cooperative interleaving on one thread
//! saturates the useful concurrency — parallelism across sessions buys nothing. This loop needs no
//! `Send`; sessions never cross a thread.
//!
//! **The kernel-async seam:** `AgentHost::deliver`/`fire_due_timers` are ASYNC (they drive the kernel's
//! `Session::deliver`/`fire_due_timers`, folding through an [`cdz_kernel::reducer::Reducer`]
//! with `Store::fuel_async_yield_interval`), so a long fold cooperatively YIELDS and this loop interleaves
//! other sessions while it awaits — SAME loop, no reshape, no `Send` (still one task). The in-place
//! `host.deliver(..).await` here is exactly that seam.

use crate::admin::{AdminAuthorizer, AdminCommand, AdminResponse, AllowList, SessionFactory};
use crate::host::{AgentHost, SessionId};
use crate::lifecycle::{LifecycleChannel, LifecycleOp};
use cdz_kernel::effect::Payload;
use cdz_kernel::event::{ContentType, EventBody};
use cdz_kernel::hash::Hash;
use cdz_kernel::kernel::KernelError;
use tokio::sync::{mpsc, oneshot};

/// The content-type family of a BOUNCE event (§lifecycle I5): when a cross-session [`Emit`](crate::EmitExecutor)
/// targets a session that is GONE (terminated → removed from the registry) or terminated-in-place, the loop
/// delivers an [`EventBody::Inbound`] of this family back to the ORIGINATING session (its `reply_to`), so the
/// sender's reducer observes the failure (a Failure-to-sender, §effect-model — never a silent drop). The
/// payload echoes the FAILED MESSAGE'S OWN BYTES (the guest-opaque payload it emitted, §4) so the sender
/// correlates the bounce to the specific Emit. The human-readable REASON is log-trace only (recorded at the
/// bounce call site), NOT in the guest payload — a reducer keys off its own echoed bytes, not a reason string.
const DELIVERY_FAILURE_FAMILY: &str = "delivery-failure";
/// v1 of the delivery-failure wire.
const DELIVERY_FAILURE_VERSION: u32 = 1;

/// Backpressure ceiling for the suspend HOLD buffer (§lifecycle I4 / #2452 Copilot c2). Suspend is
/// SHORT-LIVED BY CONTRACT — a supervisor suspends a session to inspect/migrate/throttle it, not to park
/// it indefinitely — so the held-inbound buffer is bounded in normal operation. But nothing STRUCTURALLY
/// bounded it: the inbox is unbounded, so a misbehaving/long-suspend target with live producers could grow
/// `held_inbound` without limit → host OOM. This cap makes the bound explicit + DEFENSIVE: once a single
/// loop's held buffer reaches it, the loop SHEDS the oldest held message (bouncing it if it was a
/// cross-session Emit, so the sender learns; dropping a producer-less external inbound) rather than
/// accumulate unboundedly. A generous ceiling — a healthy short suspension never approaches it; hitting it
/// signals a stuck-suspended target or a flooding producer, both of which shedding + the warn surface.
const HELD_INBOUND_CAP: usize = 4096;

/// One inbound delivery to route to a session: its id + the event body + optional cause. The `Inbox`
/// sender clones cheaply, so many producers (network listeners, peer-emit bridges, a test) can feed the
/// one loop.
pub struct Inbound {
    pub session: SessionId,
    pub body: EventBody,
    pub cause: Option<Hash>,
    /// HOST-INTERNAL routing return-address (§lifecycle I5): the id of the session that ORIGINATED this
    /// delivery, when it was produced by a cross-session [`Emit`](crate::EmitExecutor) — so if the target is
    /// gone (terminated → removed from the registry) or terminated-in-place, the loop can BOUNCE a
    /// `delivery-failure` back to the originator (a Failure-to-sender, not a silent drop). `None` for an
    /// ordinary external inbound (a network/admin producer with no originating session to bounce to).
    ///
    /// This is host ROUTING METADATA, never a kernel field and never guest-interpreted: the guest payload
    /// stays opaque (§4), and the origin is the host's own dispatch context (the sender IS the session being
    /// driven when its reducer emitted). It is NOT persisted into the target's log — only `body`/`cause` are.
    pub reply_to: Option<SessionId>,
}

/// The sending half a producer holds to deliver events into the host loop. Cloneable (mpsc sender).
pub type Inbox = mpsc::UnboundedSender<Inbound>;

/// The correlation payload to echo in a bounce (§lifecycle I5): the failed message's OWN payload, so the
/// sender's reducer can match the delivery-failure to the specific message it emitted. For an
/// [`EventBody::Inbound`] (what a cross-session Emit produces) that's its `payload`; any other body (a
/// bounce should only ever originate from an Emit-produced Inbound) yields an empty payload.
fn bounce_echo_payload(body: &EventBody) -> Payload {
    match body {
        EventBody::Inbound { payload, .. } => payload.clone(),
        _ => Payload::Inline(Vec::new().into()),
    }
}

/// Route a `delivery-failure` bounce back to the ORIGINATING session (§lifecycle I5): a cross-session Emit
/// could not be delivered because its target is gone (terminated → removed) or terminated-in-place, so the
/// sender's reducer folds this Inbound as a Failure-to-sender instead of the emit silently vanishing. The
/// bounce is delivered IN-PLACE (same `host.deliver` path) — the sender is a live registered session on this
/// loop. Best-effort by design: if the SENDER is itself gone/terminated (its own delivery returns
/// `None`/`FoldRefused`) there is nothing to notify and we drop the bounce (no bounce-of-a-bounce); a real
/// `KernelError` on the sender's fold still fails the loop fast (propagated via `?`).
async fn bounce_delivery_failure(
    host: &mut AgentHost,
    sender: &SessionId,
    failed_target: &SessionId,
    failed_payload: Payload,
    reason: &str,
) -> Result<(), KernelError> {
    // Guest-opaque payload: the sender defined the message schema, so we echo its own bytes back for
    // correlation (§4 — the host never interprets it). The `reason` rides the log's tracing, not the guest
    // payload (guest sees its own message came back under the delivery-failure family).
    //
    // Trace the failure for operator diagnosis (#2409 Copilot c1). SAFE to log: `sender`/`failed_target` are
    // host-controlled SessionId metadata (an opaque routing id or a genesis-hash-hex — either way host-, not
    // guest-, authored) and `reason` is a host-authored delivery-failure cause (absent-target / FoldRefused) —
    // none is guest-controlled payload, so no guest-string-logging concern (§4 keeps the echoed payload out of
    // the log entirely).
    tracing::warn!(
        sender = %sender.to_hex(),
        failed_target = %failed_target.to_hex(),
        reason = %reason,
        "delivery-failure: bouncing an undeliverable cross-session Emit back to its sender"
    );
    let body = EventBody::Inbound {
        content_type: ContentType {
            family: DELIVERY_FAILURE_FAMILY.into(),
            version: DELIVERY_FAILURE_VERSION,
        },
        payload: failed_payload,
    };
    match host.deliver(sender, body, None).await {
        // Sender folded the bounce, or the sender is itself gone/terminated → nothing more to do (no
        // bounce-of-a-bounce). Only a genuine KernelError on the sender's own fold propagates.
        Some(Ok(())) | None | Some(Err(KernelError::FoldRefused)) => Ok(()),
        Some(Err(e)) => Err(e),
    }
}

/// Drain + APPLY the lifecycle ops a session's [`LifecycleExecutor`](crate::LifecycleExecutor) recorded
/// during a `deliver` (§lifecycle I5 defer-to-loop). Called after each loop iteration's deliver, where
/// `&mut host` is free (the executor couldn't mutate the registry from inside `perform`). Drains
/// synchronously (`try_recv` — the ops were produced on this same task, nothing to await).
///
/// `Terminate` → [`AgentHost::terminate`](crate::AgentHost::terminate) (append the durable `Terminated`
/// marker + remove from the registry). `Some(Ok(_))` = terminated; `Some(Err(FoldRefused))` = the target
/// was already terminated (benign double-terminate); `None` = no such session (benign — already gone). A
/// REAL `KernelError` (not `FoldRefused`) is corruption → propagate (fail fast, like the deliver arm).
///
/// The `by` on the op is the controller's SessionId string, which IS its genesis-hash-HEX (operator ruling).
/// The durable `Terminated{by}` marker wants the controller's actual genesis `Hash` — so PARSE the hex back
/// with `Hash::from_hex` (round-trips the id to the real Hash), NOT `Hash::of` (which would HASH the hex TEXT
/// → a different Hash no consumer/authz could match against the controller's identity). A non-hex SessionId
/// (a test/legacy string id, not a genesis hash) can't round-trip → fall back to `Hash::of` of its bytes (a
/// stable opaque tag; those ids aren't real genesis hashes anyone matches against anyway).
async fn apply_lifecycle_ops(
    host: &mut AgentHost,
    lifecycle_rx: &mut mpsc::UnboundedReceiver<LifecycleOp>,
    factory: Option<&mut (dyn SessionFactory + '_)>,
    #[cfg(feature = "live-aws-storage")] session_registry: Option<
        &crate::session_registry::DynamoSessionRegistry,
    >,
    #[cfg(feature = "live-aws-storage")] now_ms: u64,
) -> Result<(), KernelError> {
    // The factory is consumed by at most one Spawn op per drain in practice, but a single drain can carry
    // several ops; re-borrow per Spawn via `as_deref_mut` on an Option we hold across the loop.
    let mut factory = factory;
    while let Ok(op) = lifecycle_rx.try_recv() {
        match op {
            LifecycleOp::Terminate { target, by, reason } => {
                // `by` = the controller's genesis hash (recorded as who terminated). A SessionId IS the
                // genesis Hash now (operator ruling), so it's the controller's genesis directly — no
                // registry lookup / hex parse / Hash::of fallback needed (the pre-Rule-A dance for an opaque
                // string id that might be a vanity name or a hex hash).
                let by_hash = by.hash();
                match host.terminate(&target, by_hash, reason).await {
                    // Terminated, or a benign no-op (already-terminated FoldRefused / absent None) — nothing
                    // more to do; the durable marker (if fresh) + registry removal are done inside terminate.
                    Some(Ok(_)) | Some(Err(KernelError::FoldRefused)) | None => {
                        // I4b (slice 2c-b2): mark the session TERMINATED in the durable registry so boot-
                        // recovery skips it (an already-terminated FoldRefused is still terminated → mark it;
                        // an absent `None` session is gone — mark_terminated is idempotent + a no-op-ish put).
                        // BEST-EFFORT: a registry write failure is logged, never fails the loop (the durable
                        // log's Terminated tail is the source of truth; the registry is an index over it).
                        #[cfg(feature = "live-aws-storage")]
                        if let Some(registry) = session_registry {
                            if let Err(e) = registry.mark_terminated(&target.to_hex(), now_ms).await
                            {
                                tracing::warn!(
                                    target: "cdz_agent_host::session_registry",
                                    session_id = target.to_hex(),
                                    "session-registry mark_terminated failed (best-effort; terminate still applied): {e}"
                                );
                            }
                        }
                    }
                    // A real kernel error on the target's terminate-append is corruption — fail fast, same
                    // as the deliver arm (a reducer fault would be a FoldFailed event, not a KernelError).
                    Some(Err(e)) => return Err(e),
                }
            }
            // Suspend/resume flip the host-scheduler bit (no log mutation, so no KernelError possible). An
            // absent target (`false`) is a benign no-op — the loop just has nothing to hold/release.
            LifecycleOp::Suspend { target, by: _ } => {
                host.suspend(&target);
            }
            LifecycleOp::Resume { target, by: _ } => {
                host.resume(&target);
            }
            // Spawn: register the child the executor pre-computed. The executor already minted the nonce +
            // derived + returned the child id to the parent's reducer; the loop MATERIALIZES the reducer via
            // the session factory (build_spawned: reducer_hash → live reducer + deny-all child authz + the
            // SAME nonce) and registers it via AgentHost::spawn_child_with_nonce — so the registered id
            // matches the pre-computed one byte-for-byte + the parent→child edge is recorded.
            LifecycleOp::Spawn {
                parent,
                reducer_hash,
                spawn_nonce,
                child_id,
            } => {
                let Some(factory) = factory.as_deref_mut() else {
                    // No factory configured (a pure control-plane host with no reducer-load seam) → can't
                    // materialize a child. Loud no-op, not a silent drop; the parent already has the id but
                    // no session backs it. A deployed daemon always has a factory.
                    tracing::warn!(
                        parent = %parent.to_hex(), child_id = %child_id.to_hex(),
                        "lifecycle/spawn: no session factory configured — cannot materialize the child reducer (child NOT registered)"
                    );
                    continue;
                };
                // The parent genesis hash IS the parent SessionId (operator ruling: the id is the genesis
                // Hash), so it's a direct read — no registry lookup / hex parse / unresolvable case (the
                // pre-Rule-A dance for an opaque string id). It matches the child_id the executor
                // pre-computed (both derive from this same parent genesis).
                let parent_genesis = parent.hash();
                match factory
                    .build_spawned(reducer_hash, parent_genesis, spawn_nonce)
                    .await
                {
                    Ok(child) => {
                        // Register under the parent→child edge with the SAME nonce (the id matches the
                        // executor's pre-computed child_id). None = parent absent, Some(Err(FoldRefused)) =
                        // parent terminated — both benign (no child registered); a real KernelError fails fast.
                        match host
                            .spawn_child_prebuilt_with_nonce(
                                &parent,
                                reducer_hash,
                                spawn_nonce,
                                child,
                            )
                            .await
                        {
                            Some(Ok(registered_id)) => {
                                debug_assert_eq!(
                                    registered_id, child_id,
                                    "registered id == pre-computed"
                                );
                            }
                            Some(Err(KernelError::FoldRefused)) | None => {
                                tracing::warn!(parent = %parent.to_hex(), "lifecycle/spawn: parent gone/terminated — child not spawned (benign)");
                            }
                            Some(Err(e)) => return Err(e),
                        }
                    }
                    Err(reason) => {
                        // A malformed/absent reducer, or a factory that doesn't support spawn — loud, not a
                        // silent drop. The parent has the id but no live child (a Failure-to-parent is a
                        // follow-on refinement; v0 logs).
                        tracing::warn!(
                            parent = %parent.to_hex(), reducer_hash = %reducer_hash.to_hex(),
                            reason = %reason,
                            "lifecycle/spawn: factory could not build the child reducer — child NOT registered"
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

/// Drain + APPLY the userspace-effect reply-settle commands (design DESIGN-userspace-effects I4, loop-side).
/// A [`ReplyExecutor`](crate::reply_exec::ReplyExecutor) running in a HANDLER session validated an
/// `effect/reply` into a [`ReplySettle`](crate::reply_exec::ReplySettle) `{caller, effect_id, outcome}` and
/// enqueued it here (it can't settle the CALLER's session from inside its own `perform` — the caller's
/// reducer/authz/executor aren't in scope), the same defer-to-loop shape [`apply_lifecycle_ops`] uses for
/// `lifecycle/*`. The loop drains after each `deliver` (where `&mut host` is free) and folds each outcome onto
/// the caller's OPEN (Deferred) effect via [`AgentHost::settle_reply`](crate::host::AgentHost::settle_reply) —
/// resuming the caller's continuation, closing request→forward→reply→settle.
///
/// A settle to an ABSENT caller (gone/terminated between forward + reply) or an already-settled id is a benign
/// no-op (`settle_reply` returns `false`), logged at debug — a late/stale reply can't corrupt a log. Infallible
/// from the loop's view (a deferred-effect settle opens no new failure path — a bad settle is a no-op, not an
/// append fault), so unlike `apply_lifecycle_ops` there is no `KernelError` return.
async fn apply_reply_settles(
    host: &mut AgentHost,
    reply_settle_rx: &mut mpsc::UnboundedReceiver<crate::reply_exec::ReplySettle>,
) {
    while let Ok(settle) = reply_settle_rx.try_recv() {
        let crate::reply_exec::ReplySettle {
            caller,
            effect_id,
            outcome,
        } = settle;
        let landed = host.settle_reply(&caller, effect_id, outcome).await;
        if !landed {
            tracing::debug!(
                target: "cdz_agent_host::userspace_effect",
                caller = %caller.to_hex(),
                effect_id = effect_id.0,
                "effect/reply settle was a no-op (caller gone/terminated or effect already settled)"
            );
        }
    }
}

/// One admin command routed to the host loop with a reply channel — the in-process half of the admin
/// CONTROL INTERFACE. A producer (the future Unix-socket listener, or a test) sends this on the
/// [`AdminChannel`]; the loop applies it via [`AgentHost::apply_admin`] on the single-threaded loop task
/// (where the `!Send` registry lives) and returns the [`AdminResponse`] on `reply`.
///
/// **Why a request/reply pair (not a bare command like [`Inbound`]):** an admin command must RETURN a
/// result to the caller (installed? the session list? the status JSON?), whereas an inbound delivery is
/// fire-and-forget. The `reply` oneshot is that return path; the socket listener task (which holds the
/// `Send` half) awaits it and writes the encoded response back to the client.
pub struct AdminRequest {
    pub command: AdminCommand,
    /// The admin identity this command is submitted under — the PRINCIPAL the host's [`AdminAuthorizer`]
    /// decides on. The transport asserts it: over the v0 local `0o600` socket every caller is the daemon's
    /// owner, so the socket sets a fixed local-admin principal (the perms are the real identity gate; the
    /// authorizer scopes WHICH actions that principal may take). `None` = the transport asserted no
    /// identity → the host treats it as an anonymous principal (`""`), which a deny-by-default authorizer
    /// refuses unless explicitly granted.
    pub principal: Option<String>,
    /// The loop sends the response here. If the caller dropped the receiver (client hung up), the send
    /// fails silently — the command still applied; only the reply is discarded.
    pub reply: oneshot::Sender<AdminResponse>,
}

/// The sending half a control producer holds to submit [`AdminRequest`]s into the host loop. Cloneable.
/// This is the seam the Unix-socket listener feeds: the listener task (Send) decodes a frame into an
/// [`AdminCommand`], sends an [`AdminRequest`] here, and awaits the reply — while the single-threaded loop
/// runs the actual `apply_admin` against the non-Send registry. Clean Send/!Send split.
pub type AdminChannel = mpsc::UnboundedSender<AdminRequest>;

/// The async host: owns the [`AgentHost`] registry and runs the single-threaded multiplexing loop.
/// Construct with [`AsyncAgentHost::new`], hand out [`AsyncAgentHost::inbox`] senders to producers, then
/// [`AsyncAgentHost::run`] the loop (typically the process's main future). A `shutdown` receiver ends it.
pub struct AsyncAgentHost {
    host: AgentHost,
    rx: mpsc::UnboundedReceiver<Inbound>,
    tx: Inbox,
    /// The admin CONTROL-INTERFACE receiver — the loop drains [`AdminRequest`]s here and applies them via
    /// [`AgentHost::apply_admin`] on this (single-threaded) task. Paired with the [`AdminChannel`] sender
    /// handed to a control producer (the Unix-socket listener).
    admin_rx: mpsc::UnboundedReceiver<AdminRequest>,
    admin_tx: AdminChannel,
    /// The session factory admin `install-session` commands build through (the reducer-load seam). `None`
    /// = the host was built without one, so an `install-session` returns a clean error (a pure control
    /// plane that only lists/stops/inspects needs no factory). Held here so the loop can pass it to
    /// `apply_admin`; boxed + owned because building a session is the factory's job, not the command's.
    factory: Option<Box<dyn SessionFactory>>,
    /// The admin authorizer every admin command is gated through (deny-by-default). Defaults to
    /// [`AllowList::deny_all`] — a host that doesn't configure one refuses ALL admin commands (fail-closed:
    /// an un-configured control plane grants nothing). The deployed daemon installs a real authorizer via
    /// [`with_admin_authz`](Self::with_admin_authz) (the local-admin allowlist, or later a Cedar policy).
    admin_authz: Box<dyn AdminAuthorizer>,
    /// The `lifecycle/*` op receiver (§lifecycle I5): a session's [`LifecycleExecutor`](crate::LifecycleExecutor)
    /// RECORDS an op here (it can't mutate the registry from inside `perform` — the registry is borrowed for
    /// the session being driven), and the loop DRAINS + APPLIES it after each `deliver` (where `&mut host` is
    /// free — the defer-to-loop mechanism, same shape as `pending_admin`). Paired with the
    /// [`LifecycleChannel`] sender handed to each session's executor via [`lifecycle_channel`](Self::lifecycle_channel).
    lifecycle_rx: mpsc::UnboundedReceiver<LifecycleOp>,
    /// Retained sender for the lifecycle channel — cloned to each session's [`LifecycleExecutor`]. The loop
    /// drops its own copy at start (like `tx`/`admin_tx`) so the channel closes when the last executor drops.
    lifecycle_tx: LifecycleChannel,
    /// The userspace-effect reply-settle receiver (design DESIGN-userspace-effects I4): a session's
    /// [`ReplyExecutor`](crate::reply_exec::ReplyExecutor) enqueues a [`ReplySettle`](crate::reply_exec::ReplySettle)
    /// here (it can't settle the CALLER's session from its own `perform`), and the loop DRAINS + APPLIES it
    /// after each `deliver` via [`apply_reply_settles`] — the same defer-to-loop shape as `lifecycle_rx`.
    reply_settle_rx: mpsc::UnboundedReceiver<crate::reply_exec::ReplySettle>,
    /// Retained sender for the reply-settle channel — the daemon wires a clone into each session's
    /// [`ReplyExecutor`] (via [`LiveExecutorSet::with_userspace_effects`](crate::factory::LiveExecutorSet)). The
    /// loop drops its own copy at start so the channel closes when the last executor drops.
    reply_settle_tx: crate::reply_exec::ReplySettleSink,
    /// The durable SESSION REGISTRY (I4b), if the daemon configured one — the index the loop keeps current so
    /// boot-recovery can enumerate + re-register sessions after a restart. When present, the loop calls
    /// `register` after a successful install (below) and `mark_terminated` after a terminate (a following
    /// slice), so the registry tracks each session's lifecycle status. `None` (the default / a build with no
    /// durable registry) = the loop keeps no external index (today's lossy-on-restart behavior). Behind
    /// `live-aws-storage` (the registry's feature). Registry writes are BEST-EFFORT — a write failure is
    /// logged, never crashes the loop or fails the install (the durable log remains the source of truth;
    /// the registry is an index over it).
    #[cfg(feature = "live-aws-storage")]
    session_registry: Option<crate::session_registry::DynamoSessionRegistry>,
}

impl AsyncAgentHost {
    /// Build over an existing (already-populated) [`AgentHost`] registry, with NO session factory — an
    /// admin `install-session` then returns a clean error (suitable for a host that only lists/stops/
    /// inspects). Use [`with_factory`](Self::with_factory) to enable installs. The admin authorizer defaults
    /// to [`AllowList::deny_all`] (fail-closed); set a real one with [`with_admin_authz`](Self::with_admin_authz).
    pub fn new(host: AgentHost) -> Self {
        Self::build(host, None, Box::new(AllowList::deny_all()))
    }

    /// Build over an existing registry WITH a session factory, so admin `install-session` commands can
    /// build + register sessions at runtime (the deployed daemon's control plane). The factory is the
    /// reducer-load seam (blob-get → component → reducer, assembled host-side). Admin authorizer defaults to
    /// deny-all (fail-closed); set one with [`with_admin_authz`](Self::with_admin_authz).
    pub fn with_factory(host: AgentHost, factory: Box<dyn SessionFactory>) -> Self {
        Self::build(host, Some(factory), Box::new(AllowList::deny_all()))
    }

    /// A fresh, unconnected [`LifecycleChannel`] pair (sender, receiver) the DAEMON mints BEFORE building the
    /// factory + host, so the SAME channel can be wired into both the session executors
    /// ([`LiveExecutorSet::with_lifecycle_channel`](crate::factory::LiveExecutorSet)) and this loop (via
    /// [`with_factory_and_lifecycle`](Self::with_factory_and_lifecycle)). Solves the construction chicken-egg:
    /// the factory (which needs the sender) is built before the host, but `build` otherwise mints the channel
    /// internally. Callers/tests that don't pre-wire lifecycle executors keep using [`with_factory`] (which
    /// mints its own).
    pub fn new_lifecycle_channel() -> (LifecycleChannel, mpsc::UnboundedReceiver<LifecycleOp>) {
        mpsc::unbounded_channel()
    }

    /// Like [`with_factory`](Self::with_factory) but the loop uses a caller-supplied lifecycle channel
    /// (from [`new_lifecycle_channel`](Self::new_lifecycle_channel)) — the SAME `tx` the daemon wired into the
    /// session executors, so a session's `LifecycleExecutor` sends ops the loop actually drains. Use this
    /// (not `with_factory`) whenever the executor set was built with a lifecycle channel.
    pub fn with_factory_and_lifecycle(
        host: AgentHost,
        factory: Box<dyn SessionFactory>,
        lifecycle_tx: LifecycleChannel,
        lifecycle_rx: mpsc::UnboundedReceiver<LifecycleOp>,
    ) -> Self {
        Self::build_with_lifecycle(
            host,
            Some(factory),
            Box::new(AllowList::deny_all()),
            lifecycle_tx,
            lifecycle_rx,
        )
    }

    /// Install the admin authorizer that gates every admin command (deny-by-default). Builder-style — the
    /// deployed daemon calls e.g. `AsyncAgentHost::with_factory(..).with_admin_authz(Box::new(AllowList::allow_all_for_local_admin()))`
    /// (the trusted-local-admin preset behind the `0o600` socket) or a Cedar-policy-component authorizer.
    /// Without this, the host denies all admin commands (an un-configured control plane grants nothing).
    pub fn with_admin_authz(mut self, authz: Box<dyn AdminAuthorizer>) -> Self {
        self.admin_authz = authz;
        self
    }

    fn build(
        host: AgentHost,
        factory: Option<Box<dyn SessionFactory>>,
        admin_authz: Box<dyn AdminAuthorizer>,
    ) -> Self {
        // Mint the lifecycle channel internally (the common path — callers that don't pre-wire lifecycle
        // executors don't need to supply one).
        let (lifecycle_tx, lifecycle_rx) = mpsc::unbounded_channel();
        Self::build_with_lifecycle(host, factory, admin_authz, lifecycle_tx, lifecycle_rx)
    }

    /// The shared constructor: build with a CALLER-SUPPLIED lifecycle channel (so the daemon can wire the
    /// SAME channel into the session executors). [`build`](Self::build) mints its own + delegates here.
    fn build_with_lifecycle(
        host: AgentHost,
        factory: Option<Box<dyn SessionFactory>>,
        admin_authz: Box<dyn AdminAuthorizer>,
        lifecycle_tx: LifecycleChannel,
        lifecycle_rx: mpsc::UnboundedReceiver<LifecycleOp>,
    ) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let (admin_tx, admin_rx) = mpsc::unbounded_channel();
        let (reply_settle_tx, reply_settle_rx) = crate::reply_exec::reply_settle_channel();
        AsyncAgentHost {
            host,
            rx,
            tx,
            admin_rx,
            admin_tx,
            factory,
            admin_authz,
            lifecycle_rx,
            lifecycle_tx,
            reply_settle_rx,
            reply_settle_tx,
            #[cfg(feature = "live-aws-storage")]
            session_registry: None,
        }
    }

    /// Wire a durable SESSION REGISTRY (I4b) so the loop keeps it current: `register` on a successful install,
    /// `mark_terminated` on a terminate (following slice). Called once at daemon boot from
    /// [`SessionRegistryConfig::Dynamo`](crate::SessionRegistryConfig). Without it, the loop keeps no external
    /// session index (lossy-on-restart). Registry writes are best-effort (logged, never fail the install/loop).
    #[cfg(feature = "live-aws-storage")]
    pub fn with_session_registry(
        mut self,
        registry: crate::session_registry::DynamoSessionRegistry,
    ) -> Self {
        self.session_registry = Some(registry);
        self
    }

    /// A cloneable [`LifecycleChannel`] sender to hand each session's [`LifecycleExecutor`](crate::LifecycleExecutor)
    /// (§lifecycle I5): the executor records `lifecycle/terminate` ops on it, and the loop drains + applies
    /// them after each `deliver`. Wire it into a session's executor set at spawn/install (with that session's
    /// id as the `owner`), the way the `Emit` executor gets [`inbox`](Self::inbox).
    pub fn lifecycle_channel(&self) -> LifecycleChannel {
        self.lifecycle_tx.clone()
    }

    /// A clone of the loop's reply-settle sink (userspace-effects I4) — the daemon wires this into each
    /// session's [`ReplyExecutor`](crate::reply_exec::ReplyExecutor) via
    /// [`LiveExecutorSet::with_userspace_effects`](crate::factory::LiveExecutorSet), so a handler's
    /// `effect/reply` enqueues a [`ReplySettle`](crate::reply_exec::ReplySettle) the loop drains + folds onto
    /// the caller. Mirrors [`lifecycle_channel`](Self::lifecycle_channel).
    pub fn reply_settle_sink(&self) -> crate::reply_exec::ReplySettleSink {
        self.reply_settle_tx.clone()
    }

    /// A cloneable sender to feed inbound events into the loop. Every producer that wants to deliver to a
    /// session holds one of these.
    pub fn inbox(&self) -> Inbox {
        self.tx.clone()
    }

    /// A cloneable sender to submit admin control commands into the loop — the seam the Unix-socket
    /// listener (or a test) feeds. Each [`AdminRequest`] carries a oneshot the loop replies on.
    pub fn admin_channel(&self) -> AdminChannel {
        self.admin_tx.clone()
    }

    /// Mutable access to the registry (to `spawn` sessions before/around running the loop). During the
    /// run, spawning is done by delivering to the loop; before the run, use this.
    pub fn host_mut(&mut self) -> &mut AgentHost {
        &mut self.host
    }

    /// Read-only access to the registry — usable BEFORE `run` (which consumes `self`) or on the
    /// `AgentHost` that `run` RETURNS after shutdown. There is no concurrent `&self` while the loop runs
    /// (it owns `self`); a LIVE session-status query is routed THROUGH the event loop (a future inbound
    /// message kind), not a concurrent borrow. So this accessor serves pre-run setup + post-run inspection.
    pub fn host(&self) -> &AgentHost {
        &self.host
    }

    /// Run the single-threaded multiplexing loop until `shutdown` fires OR all inbox senders are dropped
    /// (the channel closes → no more producers → nothing left to serve). Each iteration:
    /// - FIRST fire every due timer (deadline ≤ now) across sessions — done BEFORE the `select!` so a
    ///   continuously-ready inbox can't STARVE a due deadline (a `select!` gives no fairness guarantee, so
    ///   the sleep arm might never be polled under sustained load — PR#1303; firing here bounds a timer's
    ///   lateness to one loop iteration);
    /// - then `select!` over shutdown | the next inbound event | a sleep until the earliest future deadline.
    ///
    /// `now_ms` is a monotonic clock closure so the loop stays testable (a test drives a fake clock); a
    /// real host passes wall-clock ms. Returns `Ok(AgentHost)` on clean shutdown (so the caller can inspect
    /// final state / re-run), or `Err(KernelError)` if a session's fold hit a KERNEL error — that's
    /// corruption / a programming error (a genuine reducer FAULT is instead captured as a `FoldFailed` log
    /// EVENT, not a KernelError), so the loop FAILS FAST + surfaces it rather than swallowing it (PR#1303).
    pub async fn run(
        self,
        mut shutdown: tokio::sync::oneshot::Receiver<()>,
        mut now_ms: impl FnMut() -> u64,
    ) -> Result<AgentHost, KernelError> {
        // Destructure into independent locals up front. `apply_admin` holds the factory across an `.await`
        // (the async_trait build future); a `self.factory` FIELD borrow held across that await inside the
        // loop would demand a `'static` borrow (the field is tied to `self`'s lifetime). Fully separate
        // locals — `host` + `factory` — are disjoint bindings, so the borrow is clean and each return moves
        // `host` freely. `tx`/`admin_tx` are dropped (see below).
        let AsyncAgentHost {
            mut host,
            mut rx,
            tx,
            mut admin_rx,
            admin_tx,
            mut factory,
            admin_authz,
            mut lifecycle_rx,
            lifecycle_tx,
            mut reply_settle_rx,
            reply_settle_tx,
            #[cfg(feature = "live-aws-storage")]
            session_registry,
        } = self;

        // Drop OUR retained inbox sender so the channel closes once every EXTERNAL producer drops its clone.
        // Otherwise `tx` would keep it open forever and `rx.recv()` would never return `None` — the loop
        // could only ever exit via `shutdown`. (Producers get their senders from `inbox()` BEFORE `run`.)
        drop(tx);
        // Same for the lifecycle channel: drop our retained sender so the receiver isn't kept open by the
        // loop itself (each session's LifecycleExecutor holds its own clone from `lifecycle_channel()`). We
        // DRAIN lifecycle_rx synchronously (try_recv) after each deliver rather than select! on it — a
        // lifecycle op is only ever produced BY a deliver on this same task, so there's nothing to wait for.
        drop(lifecycle_tx);
        // Same for the reply-settle channel (userspace-effects I4): drop our retained sender so it isn't kept
        // open by the loop (each session's ReplyExecutor holds its own clone from `reply_settle_sink()`).
        // DRAINED synchronously (try_recv) after each deliver like lifecycle_rx — a ReplySettle is only ever
        // produced BY a deliver on this same task (a handler's effect/reply dispatch), so nothing to await.
        drop(reply_settle_tx);
        // Same for the admin channel: drop our retained sender so `admin_rx.recv()` yields `None` once the
        // last external admin producer (the socket listener) drops its clone. The admin channel closing is
        // NOT a loop-exit condition on its own (an admin-less run still serves inbound + timers) — a closed
        // admin_rx just makes that select arm inert (recv → None).
        drop(admin_tx);
        // Once `admin_rx` closes (all admin senders dropped), its `recv()` resolves `None` IMMEDIATELY on
        // every poll — so we must stop selecting on it or the loop busy-spins. This flag latches closed and
        // gates the admin select arm off (a `select!` branch precondition), leaving inbound + timers to
        // drive the loop.
        // Each of the two producer channels latches closed independently once its last external sender
        // drops. The loop runs while EITHER is open (a pure control-plane daemon may have ONLY admin
        // producers and NO inbox producers — so a closed inbox must NOT end the loop while admin is live,
        // and vice-versa). It returns only when BOTH are closed (no producers of any kind left) or shutdown
        // fires. Gating each arm off when its channel closes also avoids busy-spinning on an immediate `None`.
        let mut inbox_open = true;
        let mut admin_open = true;
        // A slot for an admin request captured in the select! and handled just below it (see the admin arm).
        let mut pending_admin: Option<AdminRequest> = None;
        // Inbound HELD because its target session is suspended (§lifecycle I4): not delivered, not dropped —
        // replayed when the target resumes (drained after each apply_lifecycle_ops, which may have resumed
        // it). Bounded by `HELD_INBOUND_CAP` at the push site (#2452 c2): suspend is short-lived by contract,
        // so it stays small in practice, and the cap defends against an OOM if a target stays suspended under
        // live producers (shed-oldest-with-bounce at the ceiling).
        let mut held_inbound: Vec<Inbound> = Vec::new();
        loop {
            // Both producer channels closed → nothing can ever drive the loop again → return (unless a timer
            // is still pending, which the arms below still service; once channels are closed AND no timer is
            // armed, this exits). Checked here so a control-plane daemon whose last admin client hung up (and
            // that has no inbox producers or armed timers) shuts down cleanly.
            if !inbox_open && !admin_open && host.next_timer_deadline_across_sessions().is_none() {
                return Ok(host);
            }
            // Fire any ALREADY-DUE timers up front (deadline ≤ now), before we might block on a ready
            // inbox — this is what stops a busy inbox from starving deadlines (a `select!` has no fairness
            // guarantee). Bounds a timer's lateness to a single iteration.
            if let Some(deadline) = host.next_timer_deadline_across_sessions() {
                if deadline <= now_ms() {
                    host.fire_due_timers(now_ms()).await;
                    // Loop back: firing may have armed new timers / the inbox may now be ready; re-evaluate.
                    continue;
                }
            }

            // The next FUTURE armed-timer deadline (all due ones fired above). None = no timer armed → the
            // sleep arm never wakes; only inbound/shutdown drive the loop.
            let next_deadline = host.next_timer_deadline_across_sessions();
            let sleep = async {
                match next_deadline {
                    Some(deadline) => {
                        let dur_ms = deadline.saturating_sub(now_ms());
                        tokio::time::sleep(tokio::time::Duration::from_millis(dur_ms)).await;
                    }
                    None => std::future::pending::<()>().await,
                }
            };

            tokio::select! {
                // Shutdown wins — end the loop promptly.
                _ = &mut shutdown => return Ok(host),
                // An inbound event: route it to its session and drive that session's loop in-place. Gated on
                // `inbox_open` so a closed inbox doesn't busy-spin on an immediate `None` (mirrors the admin
                // arm). A closed inbox no longer ends the loop on its own — the admin channel may still be
                // serving (the both-closed check at the top of the loop handles final exit).
                maybe = rx.recv(), if inbox_open => {
                    match maybe {
                        Some(msg) => {
                            // §lifecycle I4: if the target session is SUSPENDED, HOLD the inbound (don't
                            // deliver, don't drop — a drop is a lossy correctness hole) — it replays when the
                            // session resumes (drained below, after apply_lifecycle_ops may have resumed it).
                            if host.is_suspended(&msg.session) {
                                // Backpressure (#2452 Copilot c2): the hold buffer is bounded in practice
                                // (suspend is short-lived by contract), but the inbox is unbounded — so cap it
                                // defensively. At the ceiling, SHED THE OLDEST held message rather than grow
                                // without bound (OOM guard): bounce it if it was a cross-session Emit (the
                                // sender learns via a delivery-failure — not a silent loss), else drop a
                                // producer-less external inbound. Warn so an operator sees a stuck-suspended
                                // target / flooding producer.
                                if held_inbound.len() >= HELD_INBOUND_CAP {
                                    let shed = held_inbound.remove(0);
                                    tracing::warn!(
                                        target_session = %shed.session.to_hex(),
                                        cap = HELD_INBOUND_CAP,
                                        "held-inbound buffer at cap — shedding the oldest held message (suspend is meant to be short-lived; a stuck-suspended target or flooding producer hit the ceiling)"
                                    );
                                    if let Some(sender) = shed.reply_to {
                                        let payload = bounce_echo_payload(&shed.body);
                                        bounce_delivery_failure(
                                            &mut host, &sender, &shed.session, payload,
                                            "held-inbound buffer overflow while target suspended (shed oldest)",
                                        )
                                        .await?;
                                    }
                                }
                                held_inbound.push(msg);
                                continue;
                            }
                            // A bounce is only ever needed for a cross-session Emit (reply_to set); an
                            // ordinary external inbound (reply_to None — the common case) never bounces. So
                            // capture the return-address + the echo payload ONLY when reply_to is set — an
                            // external inbound pays no PAYLOAD clone on the delivery hot path (Copilot #2408
                            // c2). (The `target` session-id clone just below is unconditional — `deliver`
                            // BORROWS `&msg.session`, but we still need `target` to name the failed target in
                            // the bounce AFTER `msg.body`/`msg.cause` are moved into the deliver call.)
                            // These must be taken BEFORE `msg.body` moves into deliver.
                            let bounce_ctx = msg
                                .reply_to
                                .map(|sender| (sender, bounce_echo_payload(&msg.body)));
                            let target = msg.session;
                            match host
                                .deliver_answering_signatures(&msg.session, msg.body, msg.cause, factory.as_deref_mut())
                                .await
                            {
                                // Delivered + the session ran a turn.
                                Some(Ok(())) => {}
                                // Target ABSENT from the registry. For an ORDINARY external inbound
                                // (bounce_ctx None) this is the benign stray-id no-op (a robust host doesn't
                                // crash). For a cross-session Emit (bounce_ctx set) whose target is gone —
                                // e.g. TERMINATED then removed (§lifecycle I5) — BOUNCE a delivery-failure
                                // back to the sender rather than silently dropping it (Failure-to-sender).
                                None => {
                                    if let Some((sender, payload)) = bounce_ctx {
                                        bounce_delivery_failure(
                                            &mut host, &sender, &target, payload,
                                            "target session is not registered (terminated or never spawned)",
                                        ).await?;
                                    }
                                }
                                // Target present but TERMINATED — the kernel's I1 fold guard refuses the
                                // delivery (FoldRefused). This is the ONLY KernelError that means "terminated";
                                // a cross-session Emit to it BOUNCES (Failure-to-sender), same as absent.
                                Some(Err(KernelError::FoldRefused)) => {
                                    if let Some((sender, payload)) = bounce_ctx {
                                        bounce_delivery_failure(
                                            &mut host, &sender, &target, payload,
                                            "target session is terminated (refuses further delivery)",
                                        ).await?;
                                    }
                                }
                                // Any OTHER KERNEL error (corruption / programming error — NOT a reducer
                                // fault, which is a FoldFailed event, and NOT FoldRefused). Not recoverable
                                // in-loop: fail fast so an operator/supervisor sees it (PR#1303).
                                Some(Err(e)) => return Err(e),
                            }
                        }
                        // All inbox senders dropped → stop selecting on this arm (the both-closed check at
                        // the loop top ends the loop once admin is also closed + no timer is armed).
                        None => inbox_open = false,
                    }
                }
                // An admin control command: capture it, then apply it AFTER the select! block (below) — the
                // host+factory split-borrow + await can't be held inside the select arm (it would demand a
                // 'static borrow of factory across the .await). Gated on `admin_open` so a closed channel
                // doesn't busy-spin on an immediate `None`.
                maybe = admin_rx.recv(), if admin_open => {
                    match maybe {
                        Some(req) => pending_admin = Some(req),
                        // All admin senders dropped → stop selecting on this arm (avoid the busy-spin).
                        None => admin_open = false,
                    }
                }
                // The earliest FUTURE timer came due → fire due timers across sessions (next iteration's
                // up-front check also catches any that became due meanwhile).
                _ = sleep => {
                    host.fire_due_timers(now_ms()).await;
                }
            }

            // Handle a captured admin request outside the select! (disjoint host/factory borrows are legal
            // here). Apply on THIS task (the !Send registry never leaves it) and reply on the oneshot. The
            // apply is delegated to a free `async fn` so the `factory` borrow is scoped to that call and
            // released when it returns — an inline `.await` would let the async_trait future's captured
            // lifetime inflate the borrow to the whole function, colliding with `factory`'s drop.
            if let Some(req) = pending_admin.take() {
                handle_admin(
                    &mut host,
                    factory.as_deref_mut(),
                    &*admin_authz,
                    req,
                    now_ms(),
                    #[cfg(feature = "live-aws-storage")]
                    session_registry.as_ref(),
                )
                .await;
            }

            // Apply any lifecycle ops a session's LifecycleExecutor recorded during this iteration's deliver
            // (§lifecycle I5 defer-to-loop): the executor couldn't mutate the registry from inside `perform`
            // (registry borrowed for the driven session), so it enqueued the op here — now `&mut host` is
            // free. Drained synchronously (the ops were produced on THIS task, so there's nothing to await).
            apply_lifecycle_ops(
                &mut host,
                &mut lifecycle_rx,
                factory.as_deref_mut(),
                #[cfg(feature = "live-aws-storage")]
                session_registry.as_ref(),
                #[cfg(feature = "live-aws-storage")]
                now_ms(),
            )
            .await?;

            // Apply any userspace-effect reply-settles a session's ReplyExecutor recorded this iteration
            // (userspace-effects I4 defer-to-loop): a handler's effect/reply enqueued a ReplySettle it
            // couldn't apply itself (the caller's session state isn't in scope from the handler's perform) —
            // fold each onto the caller's open Deferred effect now that `&mut host` is free. Infallible (a
            // deferred-effect settle can't corrupt the log; an absent/settled id is a benign no-op).
            apply_reply_settles(&mut host, &mut reply_settle_rx).await;

            // A lifecycle op may have RESUMED a session (§lifecycle I4): replay any inbound held for a
            // now-un-suspended target. Deliver each in-place (still-suspended ones stay held). Partition:
            // keep still-held; deliver the rest. A resumed target that TERMINATED while suspended →
            // deliver returns None (removed) / FoldRefused (marked): for a held cross-session Emit (reply_to
            // set) this BOUNCES a delivery-failure to the emitter, EXACTLY like the main inbox arm — a held
            // Emit retains its live `reply_to` (the emitter is still registered + awaiting), so dropping it
            // silently would be a lossy Failure-to-sender hole (#2452 Copilot c1 — corrected my earlier
            // "held inbound has no live emitter" claim, which was wrong: held Emits DO carry reply_to). A
            // held EXTERNAL inbound (reply_to None) still drops silently — no originator to notify.
            if !held_inbound.is_empty() {
                let mut still_held = Vec::new();
                for msg in std::mem::take(&mut held_inbound) {
                    if host.is_suspended(&msg.session) {
                        still_held.push(msg);
                        continue;
                    }
                    // Capture the bounce return-address + echo payload BEFORE msg.body moves into deliver
                    // (only when reply_to is set — a held Emit; an external inbound never bounces).
                    let bounce_ctx = msg
                        .reply_to
                        .map(|sender| (sender, bounce_echo_payload(&msg.body)));
                    let target = msg.session;
                    match host
                        .deliver_answering_signatures(
                            &msg.session,
                            msg.body,
                            msg.cause,
                            factory.as_deref_mut(),
                        )
                        .await
                    {
                        Some(Ok(())) => {}
                        // Target gone (removed) or terminated-in-place while it was suspended → bounce a
                        // delivery-failure to the held Emit's originator (Failure-to-sender), same as the
                        // main inbox arm; an external held inbound (bounce_ctx None) drops silently.
                        None => {
                            if let Some((sender, payload)) = bounce_ctx {
                                bounce_delivery_failure(
                                    &mut host, &sender, &target, payload,
                                    "held target session is not registered (terminated while suspended)",
                                )
                                .await?;
                            }
                        }
                        Some(Err(KernelError::FoldRefused)) => {
                            if let Some((sender, payload)) = bounce_ctx {
                                bounce_delivery_failure(
                                    &mut host,
                                    &sender,
                                    &target,
                                    payload,
                                    "held target session is terminated (refuses further delivery)",
                                )
                                .await?;
                            }
                        }
                        Some(Err(e)) => return Err(e),
                    }
                }
                held_inbound = still_held;
            }
        }
    }

    /// Run the loop against the system WALL CLOCK — the convenience a real host process uses (versus
    /// [`run`](Self::run), which takes an injected `now_ms` closure so tests can drive a fake clock). Timer
    /// deadlines are milliseconds since the Unix epoch, matching the `Now`/timer payload convention (§9c).
    /// Same shutdown-and-channel-close termination and the same `Result<AgentHost, KernelError>` as
    /// [`run`](Self::run) — this only supplies the clock, so a deployed daemon writes
    /// `host.run_with_wall_clock(shutdown).await` instead of hand-wiring `SystemTime`.
    ///
    /// (A wall clock can jump backward — NTP step, leap second. The loop only ever uses `now_ms` to compute
    /// a non-negative sleep duration via `saturating_sub`, so a backward jump can at worst make a timer fire
    /// slightly late, never panic or busy-spin — the same tolerance the `Now` executor's clamp relies on.)
    pub async fn run_with_wall_clock(
        self,
        shutdown: tokio::sync::oneshot::Receiver<()>,
    ) -> Result<AgentHost, KernelError> {
        self.run(shutdown, || {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                // Before the epoch = a grossly-misconfigured host clock; treat as t=0 so the loop still runs
                // (a timer just fires "immediately") rather than surfacing a clock error into the event loop.
                .unwrap_or(0)
        })
        .await
    }
}

/// AUTHORIZE then apply one admin request, replying on its oneshot — a free `async fn` (not a method) so
/// the `factory` borrow is confined to this call and released on return. Inlining the `.await` in
/// [`AsyncAgentHost::run`]'s loop would let the `async_trait` build future's captured lifetime inflate the
/// borrow to the whole function, colliding with `factory`'s end-of-scope drop.
///
/// The command is gated through `admin_authz` (deny-by-default) under the request's asserted principal
/// (`None` = anonymous `""`, which a deny-by-default authorizer refuses) BEFORE it touches the registry —
/// [`AgentHost::apply_admin_authorized`] does the authorize-then-apply, so a denied command never mutates
/// state and comes back as an error.
async fn handle_admin(
    host: &mut AgentHost,
    factory: Option<&mut (dyn SessionFactory + '_)>,
    admin_authz: &(dyn AdminAuthorizer + '_),
    req: AdminRequest,
    now_ms: u64,
    #[cfg(feature = "live-aws-storage")] session_registry: Option<
        &crate::session_registry::DynamoSessionRegistry,
    >,
) {
    let principal = req.principal.as_deref().unwrap_or("");
    // I4b: capture the install's reducer hash BEFORE `apply_admin_authorized` consumes the command, so a
    // successful install can REGISTER the session in the durable registry (the index boot-recovery reads).
    #[cfg(feature = "live-aws-storage")]
    let install_reducer_hash = match &req.command {
        AdminCommand::InstallSession(spec) => Some(spec.reducer_hash),
        _ => None,
    };
    let resp = host
        .apply_admin_authorized(req.command, principal, admin_authz, factory, Some(now_ms))
        .await;
    // On a successful install, register the session (status=active) in the durable registry — BEST-EFFORT: a
    // write failure is logged, NEVER fails the install (the durable log is the source of truth; the registry
    // is an index over it, rebuilt-able). The reducer hash is what boot-recovery reloads the reducer by.
    #[cfg(feature = "live-aws-storage")]
    if let (Some(registry), AdminResponse::Installed { id }, Some(reducer_hash)) =
        (session_registry, &resp, &install_reducer_hash)
    {
        if let Err(e) = registry.register(&id.to_hex(), *reducer_hash, now_ms).await {
            // Non-sensitive: the session id + a registry-write error. Never a guest-controlled string.
            tracing::warn!(
                target: "cdz_agent_host::session_registry",
                session_id = id.to_hex(),
                "session-registry register failed (best-effort; install still succeeded): {e}"
            );
        }
    }
    // The caller may have hung up (dropped the receiver); the command still applied, so a failed
    // reply-send is fine to ignore.
    let _ = req.reply.send(resp);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::HostedSession;
    use crate::ClockExecutor;
    use cdz_kernel::authz::Authorizer;
    use cdz_kernel::effect::{
        effect_ct, Capability, EffectKind, EffectRequest, Payload, ResourcePredicate, Timeliness,
    };
    use cdz_kernel::event::{ContentType, EffectOutcome, Event};
    use cdz_kernel::executor::CompositeExecutor;
    use cdz_kernel::kv::Kv;
    use cdz_kernel::reducer::{FoldOutput, Reducer};

    /// On "go", record which session ran by stamping "ran" in KV (via a Now round-trip so it exercises
    /// the real executor path through the loop).
    struct MarkAgent;
    #[async_trait::async_trait(?Send)]
    impl Reducer for MarkAgent {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    FoldOutput::with(vec![EffectRequest::new_with_family(
                        effect_ct::NOW,
                        String::new(),
                        None,
                        Timeliness::Interactive,
                    )])
                }
                EventBody::EffectResult {
                    result: EffectOutcome::Ok(_),
                    ..
                } => {
                    kv.put(b"ran".to_vec(), b"1".to_vec());
                    FoldOutput::none()
                }
                _ => FoldOutput::none(),
            }
        }
    }

    fn mark_host() -> HostedSession {
        let executor = cdz_kernel::executor::CompositeExecutor::new()
            .with_effect(effect_ct::NOW, Box::new(ClockExecutor::new()));
        HostedSession::genesis(
            Hash::of(b"mark-v1"),
            Box::new(MarkAgent),
            Box::new(Authorizer::new(vec![Capability {
                kind: EffectKind::Now,
                predicate: ResourcePredicate::Any,
            }])),
            executor,
        )
    }

    fn go() -> EventBody {
        EventBody::Inbound {
            content_type: ContentType {
                family: "message".into(),
                version: 1,
            },
            payload: Payload::Inline(b"go".to_vec().into()),
        }
    }

    #[tokio::test]
    async fn two_sessions_interleave_on_one_loop() {
        // Two sessions registered; both fed an inbound via the shared inbox; the single loop drives each
        // in turn. After both are delivered + the senders dropped, the loop ends and BOTH ran.
        let mut host = AgentHost::new();
        host.spawn(SessionId::new(Hash::of(b"a")), mark_host());
        host.spawn(SessionId::new(Hash::of(b"b")), mark_host());
        let async_host = AsyncAgentHost::new(host);
        let inbox = async_host.inbox();

        let (_sd_tx, sd_rx) = tokio::sync::oneshot::channel();
        // Feed both sessions, then DROP the inbox → the loop drains the two messages and returns (all
        // senders gone). A fixed clock (no timers armed here) keeps it deterministic.
        inbox
            .send(Inbound {
                session: SessionId::new(Hash::of(b"a")),
                body: go(),
                cause: None,
                reply_to: None,
            })
            .unwrap();
        inbox
            .send(Inbound {
                session: SessionId::new(Hash::of(b"b")),
                body: go(),
                cause: None,
                reply_to: None,
            })
            .unwrap();
        drop(inbox);

        // Run to completion (channel closes after the two messages) → returns the registry to inspect.
        let host = async_host
            .run(sd_rx, || 0)
            .await
            .expect("clean shutdown, no kernel error");

        // Both sessions ran their loop through the real ClockExecutor.
        for id in ["a", "b"] {
            assert_eq!(
                host.get(&SessionId::new(Hash::of(id.as_bytes())))
                    .unwrap()
                    .session()
                    .kv()
                    .get(b"ran"),
                Some(&b"1"[..]),
                "session {id} ran"
            );
        }
    }

    #[tokio::test]
    async fn shutdown_ends_the_loop_even_with_live_senders() {
        // With a live inbox sender (loop would otherwise wait for input), firing shutdown ends run()
        // promptly — graceful shutdown.
        let host = AgentHost::new();
        let async_host = AsyncAgentHost::new(host);
        let _inbox = async_host.inbox(); // kept alive → channel stays open
        let (sd_tx, sd_rx) = tokio::sync::oneshot::channel();
        sd_tx.send(()).unwrap();
        // Should return via the shutdown arm, not hang.
        let _host = async_host
            .run(sd_rx, || 0)
            .await
            .expect("shutdown returns Ok");
    }

    /// An EMITTER agent (cross-session messaging, sender half): on an inbound "trigger" it performs an
    /// `Emit` effect to a fixed peer session id, carrying a fixed message payload. `target` is the raw peer
    /// SessionId string (the wire contract); the emit is fire-and-forget (the result folds to nothing).
    /// On "go", performs a `lifecycle/terminate` targeting `victim` (the peer-control path, §lifecycle I5).
    struct TerminatorAgent {
        victim: String,
    }
    #[async_trait::async_trait(?Send)]
    impl Reducer for TerminatorAgent {
        async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    FoldOutput::with(vec![EffectRequest::new_with_family(
                        effect_ct::LIFECYCLE_TERMINATE,
                        self.victim.clone(),
                        Some(Payload::Inline(b"kill".to_vec().into())),
                        Timeliness::Interactive,
                    )])
                }
                _ => FoldOutput::none(),
            }
        }
    }

    /// A parent that SPAWNS a child on inbound: performs `lifecycle/spawn` with `reducer_hash` as the 32-byte
    /// inline payload, and records the returned child id (the effect result) into KV["child"].
    struct SpawnerAgent {
        child_reducer: Hash,
    }
    #[async_trait::async_trait(?Send)]
    impl Reducer for SpawnerAgent {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    FoldOutput::with(vec![EffectRequest::new_with_family(
                        effect_ct::LIFECYCLE_SPAWN,
                        String::new(), // spawn has no peer target
                        Some(Payload::Inline(
                            self.child_reducer.as_bytes().to_vec().into(),
                        )),
                        Timeliness::Interactive,
                    )])
                }
                // The spawn effect result carries the child SessionId (the pre-computed id) — record it.
                EventBody::EffectResult {
                    result: EffectOutcome::Ok(Some(Payload::Inline(bytes))),
                    ..
                } => {
                    kv.put(b"child".to_vec(), bytes.to_vec());
                    FoldOutput::none()
                }
                _ => FoldOutput::none(),
            }
        }
    }

    struct EmitterAgent {
        peer: String,
        message: Vec<u8>,
    }
    #[async_trait::async_trait(?Send)]
    impl Reducer for EmitterAgent {
        async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    FoldOutput::with(vec![EffectRequest::new_with_family(
                        effect_ct::EMIT,
                        self.peer.clone(),
                        Some(Payload::Inline(self.message.clone().into())),
                        Timeliness::Interactive,
                    )])
                }
                _ => FoldOutput::none(),
            }
        }
    }

    /// A RECEIVER agent (cross-session messaging, target half): folds an inbound `message` (the routed peer
    /// signal) into KV under `inbox` — the observable state change proving the message arrived + was folded.
    struct ReceiverAgent;
    #[async_trait::async_trait(?Send)]
    impl Reducer for ReceiverAgent {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
            if let EventBody::Inbound {
                content_type,
                payload: Payload::Inline(bytes),
            } = &event.body
            {
                if content_type.matches_family("message") {
                    kv.put(b"inbox".to_vec(), bytes.to_vec());
                }
            }
            FoldOutput::none()
        }
    }

    #[tokio::test]
    async fn cross_session_emit_routes_a_message_from_a_to_b() {
        // THE cross-session messaging E2E (operator's "next"): session A emits to B → the host EmitExecutor
        // routes the signal → it lands as an Inbound in B → B folds it into KV.
        //
        // Driven DIRECTLY (not under the full AsyncAgentHost loop): the loop routes an Inbound by dequeuing
        // it from the shared Inbox and calling `deliver` on the target — the trivial glue that
        // `two_sessions_interleave_on_one_loop` already covers. Here the test plays that role so completion
        // is deterministic (a fire-and-forget loop has no clean "done" signal — the EmitExecutor's retained
        // Inbox clone keeps the channel open). What's exercised REALLY: A's full turn
        // (deliver→fold→emit→authorize→dispatch via the REAL EmitExecutor), the EmitExecutor's routing (it
        // constructs the peer Inbound + sends it on the real Inbox), and B's real fold of the routed message.
        let (tx, mut rx) = mpsc::unbounded_channel::<Inbound>();

        // The two peers, addressed by their canonical genesis-hash hex (the emit target + authz predicate
        // are the same hex the EmitExecutor parses back to a SessionId).
        let session_a = SessionId::new(Hash::of(b"session-a"));
        let session_b = SessionId::new(Hash::of(b"session-b"));

        // A: EmitterAgent → emits to B on its trigger, dispatched by the REAL EmitExecutor over `tx`.
        // AUTHORIZED to Emit exactly to B (the kernel gates the emit before dispatch, SEC-F1 — an un-granted
        // target would be DENIED, and this proves the AUTHORIZED path).
        let mut a = HostedSession::genesis(
            Hash::of(b"emitter-v1"),
            Box::new(EmitterAgent {
                peer: session_b.to_hex(),
                message: b"hello-from-a".to_vec(),
            }),
            Box::new(Authorizer::new(vec![Capability {
                kind: EffectKind::Emit,
                predicate: ResourcePredicate::Exact(session_b.to_hex().into()),
            }])),
            CompositeExecutor::new().with_effect(
                effect_ct::EMIT,
                Box::new(crate::EmitExecutor::new(tx.clone(), session_a)),
            ),
        );
        // B: ReceiverAgent → folds the routed message into KV. Performs no effects (deny-all authz is fine).
        let mut b = HostedSession::genesis(
            Hash::of(b"receiver-v1"),
            Box::new(ReceiverAgent),
            Box::new(Authorizer::deny_all()),
            CompositeExecutor::new(),
        );

        // Drive A's turn: inbound trigger → A folds → emits Emit(target=session-b) → authorized → the
        // EmitExecutor routes a peer Inbound onto `tx`. A's turn runs to completion (the emit's Ok(None)
        // result folds to nothing — fire-and-forget).
        a.deliver(go(), None)
            .await
            .expect("A runs its turn (fold → emit → authorized → routed) without a kernel error");
        assert_eq!(
            a.open_effects(),
            0,
            "A's emit dispatched + its Ok(None) result folded back"
        );

        // The EmitExecutor routed exactly one peer Inbound, addressed to B with family "message" + A's
        // payload verbatim — the routing contract.
        let routed = rx
            .try_recv()
            .expect("A's Emit routed a peer Inbound onto the inbox");
        assert_eq!(
            routed.session, session_b,
            "routed to the target peer session"
        );
        // The routed Inbound carries the EMITTER as reply_to (§lifecycle I5): this is the return-address the
        // loop bounces a delivery-failure to if B is gone/terminated. Assert it so a regression that drops or
        // mis-stamps reply_to (bounces would stop reaching the emitter) is caught (Copilot #2408 c3).
        assert_eq!(
            routed.reply_to,
            Some(session_a),
            "the routed Inbound stamps the emitter as reply_to (the bounce return-address)"
        );
        // Exactly one message was routed: the channel is now EMPTY (not disconnected — the EmitExecutor
        // still holds a cloned sender, and so does this test's `tx`). Assert on `Empty` specifically so a
        // future spurious extra emit (a second queued message) is caught, distinct from a Disconnected
        // channel (#2356 review — `is_err()` blurred the two).
        assert!(
            matches!(rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
            "exactly one message routed (channel empty, no spurious extra)"
        );

        // Deliver the routed Inbound to B (the loop's routing step) → B folds it.
        b.deliver(routed.body, routed.cause)
            .await
            .expect("B folds the routed peer message without a kernel error");

        // B received A's message end to end: A.Emit → EmitExecutor route → B.Inbound → B folds → KV.
        assert_eq!(
            b.session().kv().get(b"inbox"),
            Some(&b"hello-from-a"[..]),
            "B's reducer folded the message A emitted (cross-session messaging works end to end)"
        );
    }

    /// Records a `delivery-failure` bounce (§lifecycle I5): folds a delivery-failure Inbound into KV under
    /// "bounced" with the echoed (failed-message) payload, so a test can assert the sender was notified.
    struct BounceRecorder;
    #[async_trait::async_trait(?Send)]
    impl Reducer for BounceRecorder {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
            if let EventBody::Inbound {
                content_type,
                payload: Payload::Inline(bytes),
            } = &event.body
            {
                if content_type.matches_family("delivery-failure") {
                    kv.put(b"bounced".to_vec(), bytes.to_vec());
                }
            }
            FoldOutput::none()
        }
    }

    /// COUNTS `delivery-failure` bounces (§lifecycle I4 cap test): appends one byte to KV["bounces"] per
    /// bounce folded, so a test asserts HOW MANY shed messages bounced (len = count), not just that one did.
    struct BounceCounter;
    #[async_trait::async_trait(?Send)]
    impl Reducer for BounceCounter {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
            if let EventBody::Inbound { content_type, .. } = &event.body {
                if content_type.matches_family("delivery-failure") {
                    let mut acc = kv.get(b"bounces").map(|v| v.to_vec()).unwrap_or_default();
                    acc.push(1);
                    kv.put(b"bounces".to_vec(), acc);
                }
            }
            FoldOutput::none()
        }
    }

    #[tokio::test]
    async fn an_emit_to_an_absent_target_bounces_a_delivery_failure_to_the_sender() {
        // §lifecycle I5 bounce: an Inbound produced by a cross-session Emit (reply_to = the sender) whose
        // TARGET is not registered (terminated→removed, or never spawned) does NOT silently drop — the loop
        // routes a `delivery-failure` Inbound back to the sender, which folds it. Drive the real loop.
        let mut host = AgentHost::new();
        // Only the SENDER is registered; the target "ghost" is absent (models a terminated-then-removed peer).
        host.spawn(SessionId::new(Hash::of(b"sender")), {
            HostedSession::genesis(
                Hash::of(b"bounce-recorder-v1"),
                Box::new(BounceRecorder),
                Box::new(Authorizer::deny_all()),
                CompositeExecutor::new(),
            )
        });
        let async_host = AsyncAgentHost::new(host);
        let inbox = async_host.inbox();
        let (_sd_tx, sd_rx) = tokio::sync::oneshot::channel();

        // A message addressed to the ABSENT target, carrying the sender as reply_to (what EmitExecutor
        // stamps). The loop's deliver → None (absent) → bounce a delivery-failure to "sender".
        inbox
            .send(Inbound {
                session: SessionId::new(Hash::of(b"ghost")),
                body: EventBody::Inbound {
                    content_type: ContentType {
                        family: "message".into(),
                        version: 1,
                    },
                    payload: Payload::Inline(b"undeliverable".to_vec().into()),
                },
                cause: None,
                reply_to: Some(SessionId::new(Hash::of(b"sender"))),
            })
            .unwrap();
        drop(inbox);

        let host = async_host
            .run(sd_rx, || 0)
            .await
            .expect("clean shutdown, no kernel error");

        // The sender folded a delivery-failure carrying the failed message's payload (correlation echo).
        assert_eq!(
            host.get(&SessionId::new(Hash::of(b"sender")))
                .unwrap()
                .session()
                .kv()
                .get(b"bounced"),
            Some(&b"undeliverable"[..]),
            "the sender folded a delivery-failure bounce for its undeliverable Emit (not a silent drop)"
        );
    }

    #[tokio::test]
    async fn an_external_inbound_to_an_absent_target_is_a_silent_noop_no_bounce() {
        // The bounce is ONLY for cross-session Emits (reply_to set). An ORDINARY external inbound (reply_to
        // None) to an absent id stays the benign stray-id no-op — no bounce, no panic, clean loop exit.
        let mut host = AgentHost::new();
        host.spawn(SessionId::new(Hash::of(b"live")), mark_host());
        let async_host = AsyncAgentHost::new(host);
        let inbox = async_host.inbox();
        let (_sd_tx, sd_rx) = tokio::sync::oneshot::channel();
        inbox
            .send(Inbound {
                session: SessionId::new(Hash::of(b"nobody")),
                body: go(),
                cause: None,
                reply_to: None, // external producer, no originating session → no bounce
            })
            .unwrap();
        drop(inbox);
        let host = async_host
            .run(sd_rx, || 0)
            .await
            .expect("a stray external inbound is a clean no-op, not a loop error");
        // The live session is untouched (it was never addressed); nothing bounced anywhere.
        assert!(host.contains(&SessionId::new(Hash::of(b"live"))));
    }

    #[tokio::test]
    async fn lifecycle_terminate_e2e_controller_terminates_a_peer_through_the_loop() {
        // §lifecycle I5 slice-3 E2E (defer-to-loop): a controller session performs lifecycle/terminate(victim)
        // → its LifecycleExecutor records the op on the lifecycle channel + returns Ok(None) → the loop drains
        // the op AFTER the controller's deliver + drives AgentHost::terminate → the victim is marked Terminated
        // + removed from the registry. Proves the executor→channel→loop-apply path end to end.
        let mut async_host = AsyncAgentHost::new(AgentHost::new());
        // The victim: an ordinary session (deny-all authz is fine; it performs nothing).
        async_host.host_mut().spawn(
            SessionId::new(Hash::of(b"victim")),
            HostedSession::genesis(
                Hash::of(b"victim-v1"),
                Box::new(MarkAgent),
                Box::new(Authorizer::deny_all()),
                CompositeExecutor::new(),
            ),
        );
        // The CONTROLLER: a LifecycleExecutor over THIS loop's lifecycle channel (owner = "controller"),
        // authorized to lifecycle/terminate the victim. Fetched after wrapping (the channel lives on the loop).
        let victim_hex = SessionId::new(Hash::of(b"victim")).to_hex();
        let controller = HostedSession::genesis(
            Hash::of(b"controller-v1"),
            Box::new(TerminatorAgent {
                victim: victim_hex.clone(),
            }),
            // lifecycle/* is a register-by-string family (no dedicated EffectKind) → grant it via a
            // FAMILY grant (Capability::for_family), not a kind-based Capability. Authorized to
            // lifecycle/terminate exactly the victim (by its canonical genesis-hash hex).
            Box::new(
                Authorizer::new(vec![]).with_family_grants(vec![Capability::for_family(
                    effect_ct::LIFECYCLE_TERMINATE,
                    ResourcePredicate::Exact(victim_hex.into()),
                )]),
            ),
            CompositeExecutor::new().with_effect(
                effect_ct::LIFECYCLE_TERMINATE,
                Box::new(crate::LifecycleExecutor::new(
                    async_host.lifecycle_channel(),
                    SessionId::new(Hash::of(b"controller")),
                    Hash::of(b"controller"),
                )),
            ),
        );
        async_host
            .host_mut()
            .spawn(SessionId::new(Hash::of(b"controller")), controller);

        let inbox = async_host.inbox();
        let (_sd_tx, sd_rx) = tokio::sync::oneshot::channel();
        // Trigger the controller → it emits lifecycle/terminate(victim). Drop inbox → loop drains + returns.
        inbox
            .send(Inbound {
                session: SessionId::new(Hash::of(b"controller")),
                body: go(),
                cause: None,
                reply_to: None,
            })
            .unwrap();
        drop(inbox);
        let host = async_host
            .run(sd_rx, || 0)
            .await
            .expect("clean shutdown, no kernel error");
        assert!(
            !host.contains(&SessionId::new(Hash::of(b"victim"))),
            "the victim was terminated + removed from the registry via lifecycle/terminate through the loop"
        );
        // The controller itself is untouched (it terminated a PEER, not itself).
        assert!(host.contains(&SessionId::new(Hash::of(b"controller"))));
    }

    #[tokio::test]
    async fn lifecycle_terminate_through_loop_auto_evicts_the_victim_from_its_groups_i5() {
        // §directory-I5 FULL LOOP E2E (the host-direct unit test's end-to-end peer): a controller performs
        // lifecycle/terminate(victim) THROUGH the async loop → apply_lifecycle_ops drives AgentHost::terminate
        // → I5 scan-on-death auto-evicts the victim from every canonical-store group it was in, while a
        // survivor stays. Proves terminate→I5-eviction over the real executor→channel→loop-apply path.
        use cdz_kernel::name_store::{MemberOp, NameStore};
        const GROUP: &str = "session/room/lobby";

        // The victim must be registered under its GENESIS-HASH-HEX id (I5 parses the SessionId back to the
        // member Hash), and be a member of a group in the host-owned canonical store.
        let victim_session = HostedSession::genesis(
            Hash::of(b"victim-i5-v1"),
            Box::new(MarkAgent),
            Box::new(Authorizer::deny_all()),
            CompositeExecutor::new(),
        );
        let victim_hash = victim_session.genesis_hash();
        let victim_id = SessionId::new(victim_hash);
        let survivor_hash = Hash::of(b"survivor-i5");
        let origin = Hash::of(b"adder-origin");

        let mut canonical = NameStore::new();
        canonical
            .add_op(GROUP, MemberOp::add(victim_hash, origin, 0))
            .unwrap();
        canonical
            .add_op(GROUP, MemberOp::add(survivor_hash, origin, 1))
            .unwrap();

        let mut async_host = AsyncAgentHost::new(AgentHost::with_canonical_store(canonical));
        async_host.host_mut().spawn(victim_id, victim_session);
        // The controller: authorized to lifecycle/terminate the victim (by its hex id), wired to this loop.
        let controller = HostedSession::genesis(
            Hash::of(b"controller-i5-v1"),
            Box::new(TerminatorAgent {
                victim: victim_id.to_hex(),
            }),
            Box::new(
                Authorizer::new(vec![]).with_family_grants(vec![Capability::for_family(
                    effect_ct::LIFECYCLE_TERMINATE,
                    ResourcePredicate::Exact(victim_id.to_hex().into()),
                )]),
            ),
            CompositeExecutor::new().with_effect(
                effect_ct::LIFECYCLE_TERMINATE,
                Box::new(crate::LifecycleExecutor::new(
                    async_host.lifecycle_channel(),
                    SessionId::new(Hash::of(b"controller")),
                    Hash::of(b"controller"),
                )),
            ),
        );
        async_host
            .host_mut()
            .spawn(SessionId::new(Hash::of(b"controller")), controller);

        let inbox = async_host.inbox();
        let (_sd_tx, sd_rx) = tokio::sync::oneshot::channel();
        inbox
            .send(Inbound {
                session: SessionId::new(Hash::of(b"controller")),
                body: go(),
                cause: None,
                reply_to: None,
            })
            .unwrap();
        drop(inbox);
        let host = async_host
            .run(sd_rx, || 0)
            .await
            .expect("clean shutdown, no kernel error");

        // Victim terminated + removed.
        assert!(!host.contains(&victim_id), "victim terminated via the loop");
        // I5: the victim is auto-evicted from the group; the survivor stays.
        let members = host
            .canonical_store()
            .unwrap()
            .borrow()
            .resolve_all(GROUP)
            .unwrap();
        assert!(
            !members.contains(&victim_hash),
            "terminate-through-the-loop auto-evicted the victim from its group (I5 end to end)"
        );
        assert!(
            members.contains(&survivor_hash),
            "the survivor stays in the group"
        );
    }

    #[tokio::test]
    async fn lifecycle_spawn_e2e_a_parent_spawns_a_child_through_the_loop() {
        // §lifecycle I3 FULL E2E: a parent performs lifecycle/spawn → the executor pre-computes + returns the
        // child id (parent folds it) + records LifecycleOp::Spawn → the loop's apply-step materializes the
        // child via the factory's build_spawned + registers it via spawn_child_prebuilt_with_nonce. Asserts:
        // the child is registered under the pre-computed id, the parent→child edge is on the parent's log,
        // and the id the parent folded == the registered child id (the sync-return contract holds end-to-end).
        let child_reducer = Hash::of(b"child-reducer-v1");
        // The parent's genesis hash is deterministic from its reducer + nonce; but the lifecycle executor
        // needs the parent's SessionId (= its genesis-hash-hex) at WIRING time, and the session needs that
        // executor — a chicken/egg. Resolve it: derive the parent id first (root genesis, parent=None) from a
        // FIXED nonce, wire the executor with it, then build the parent with the SAME fixed nonce so its
        // actual genesis-hash matches the id we wired.
        let parent_reducer = Hash::of(b"spawner-parent-v1");
        let parent_nonce = Hash::of(b"spawner-parent-nonce");
        let parent_id = SessionId::new(cdz_kernel::kernel::Session::derive_genesis_hash(
            parent_reducer,
            parent_nonce,
            None,
        ));

        let mut async_host = AsyncAgentHost::with_factory(AgentHost::new(), Box::new(StubFactory));
        let parent_session = HostedSession::genesis_with_nonce(
            parent_reducer,
            parent_nonce,
            Box::new(SpawnerAgent { child_reducer }),
            Box::new(
                Authorizer::new(vec![]).with_family_grants(vec![Capability::for_family(
                    effect_ct::LIFECYCLE_SPAWN,
                    ResourcePredicate::Any,
                )]),
            ),
            CompositeExecutor::new().with_effect(
                effect_ct::LIFECYCLE_SPAWN,
                Box::new(crate::LifecycleExecutor::new(
                    async_host.lifecycle_channel(),
                    parent_id,
                    cdz_kernel::kernel::Session::derive_genesis_hash(
                        parent_reducer,
                        parent_nonce,
                        None,
                    ),
                )),
            ),
        );
        async_host.host_mut().spawn(parent_id, parent_session);

        let inbox = async_host.inbox();
        let (_sd_tx, sd_rx) = tokio::sync::oneshot::channel();
        inbox
            .send(Inbound {
                session: parent_id,
                body: go(),
                cause: None,
                reply_to: None,
            })
            .unwrap();
        drop(inbox);
        let host = async_host
            .run(sd_rx, || 0)
            .await
            .expect("clean shutdown, no kernel error");

        // The parent folded the child id the executor returned — the RAW 32 genesis-hash bytes.
        let folded = host
            .get(&parent_id)
            .unwrap()
            .session()
            .kv()
            .get(b"child")
            .map(|v| v.to_vec())
            .expect("parent folded the returned child id");
        let child_hash = Hash::from_bytes(
            folded[..]
                .try_into()
                .expect("the folded child id is 32 raw genesis-hash bytes"),
        );
        let child_id = SessionId::new(child_hash);
        // The child is REGISTERED (the loop materialized + registered it via the factory) under that id.
        assert!(
            host.contains(&child_id),
            "the spawned child is registered under the pre-computed id the parent folded"
        );
        // The parent's log carries the parent→child edge, whose child_hash is the child's genesis hash (= id).
        let edges = host.get(&parent_id).unwrap().spawned_children();
        assert_eq!(edges.len(), 1, "parent recorded exactly one spawn edge");
        assert_eq!(
            edges[0], child_hash,
            "the edge's child_hash is the registered child id (sync-return contract holds end to end)"
        );
    }

    /// A controller that suspends OR resumes `target` depending on the trigger payload: an Inbound carrying
    /// `b"suspend"` emits `lifecycle/suspend(target)`, `b"resume"` emits `lifecycle/resume(target)` (any other
    /// payload is a no-op). Lets one controller drive a full suspend→(held)→resume arc through the loop.
    struct SuspendResumeController {
        target: String,
    }
    #[async_trait::async_trait(?Send)]
    impl Reducer for SuspendResumeController {
        async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            if let EventBody::Inbound {
                payload: Payload::Inline(bytes),
                ..
            } = &event.body
            {
                let family = match bytes.as_ref() {
                    b"suspend" => effect_ct::LIFECYCLE_SUSPEND,
                    b"resume" => effect_ct::LIFECYCLE_RESUME,
                    _ => return FoldOutput::none(),
                };
                return FoldOutput::with(vec![EffectRequest::new_with_family(
                    family,
                    self.target.clone(),
                    None,
                    Timeliness::Interactive,
                )]);
            }
            FoldOutput::none()
        }
    }

    /// Build a controller session wired to suspend/resume `target` through THIS loop's lifecycle channel,
    /// authorized on both families for exactly that target.
    fn suspend_resume_controller(async_host: &AsyncAgentHost, target: &str) -> HostedSession {
        // `target` is a test label; the controller addresses the peer by its canonical genesis-hash hex (the
        // same id the peer is spawned under, `Hash::of(label)`), which the LifecycleExecutor parses back.
        let target = SessionId::new(Hash::of(target.as_bytes())).to_hex();
        HostedSession::genesis(
            Hash::of(b"suspend-resume-controller-v1"),
            Box::new(SuspendResumeController {
                target: target.clone(),
            }),
            Box::new(Authorizer::new(vec![]).with_family_grants(vec![
                Capability::for_family(
                    effect_ct::LIFECYCLE_SUSPEND,
                    ResourcePredicate::Exact(target.clone().into()),
                ),
                Capability::for_family(
                    effect_ct::LIFECYCLE_RESUME,
                    ResourcePredicate::Exact(target.into()),
                ),
            ])),
            // One LifecycleExecutor per family key (CompositeExecutor dispatches by family string); both feed
            // the same loop channel.
            CompositeExecutor::new()
                .with_effect(
                    effect_ct::LIFECYCLE_SUSPEND,
                    Box::new(crate::LifecycleExecutor::new(
                        async_host.lifecycle_channel(),
                        SessionId::new(Hash::of(b"controller")),
                        Hash::of(b"controller"),
                    )),
                )
                .with_effect(
                    effect_ct::LIFECYCLE_RESUME,
                    Box::new(crate::LifecycleExecutor::new(
                        async_host.lifecycle_channel(),
                        SessionId::new(Hash::of(b"controller")),
                        Hash::of(b"controller"),
                    )),
                ),
        )
    }

    /// A message Inbound to a target session (external producer — no reply_to), carrying `payload`. `target`
    /// is a test label; the session key is its `Hash::of` genesis (the same way these tests spawn sessions).
    fn message_to(target: &str, payload: &[u8]) -> Inbound {
        Inbound {
            session: SessionId::new(Hash::of(target.as_bytes())),
            body: EventBody::Inbound {
                content_type: ContentType {
                    family: "message".into(),
                    version: 1,
                },
                payload: Payload::Inline(payload.to_vec().into()),
            },
            cause: None,
            reply_to: None,
        }
    }

    /// A controller-trigger Inbound (`b"suspend"` / `b"resume"`) delivered to the controller session.
    fn control_trigger(payload: &[u8]) -> Inbound {
        Inbound {
            session: SessionId::new(Hash::of(b"controller")),
            body: EventBody::Inbound {
                content_type: ContentType {
                    family: "message".into(),
                    version: 1,
                },
                payload: Payload::Inline(payload.to_vec().into()),
            },
            cause: None,
            reply_to: None,
        }
    }

    #[tokio::test]
    async fn held_inbound_buffer_sheds_oldest_with_a_bounce_at_the_cap() {
        // §lifecycle I4 / #2452 Copilot c2: the hold buffer is BOUNDED (HELD_INBOUND_CAP). Flooding a
        // suspended target past the cap sheds the OLDEST held message rather than growing unbounded, and
        // (since these are cross-session Emits carrying reply_to) BOUNCES each shed message to its sender —
        // not a silent loss. Drive: suspend the victim, then send CAP+N Emit-shaped inbounds for it; the
        // loop holds CAP and sheds+bounces the N oldest; the sender folds N bounces.
        const OVER: usize = 3; // how many past the cap to send
        let mut host = AgentHost::new();
        // The SENDER records every delivery-failure bounce it folds (counts them via appending a byte).
        host.spawn(
            SessionId::new(Hash::of(b"sender")),
            HostedSession::genesis(
                Hash::of(b"bounce-counter-v1"),
                Box::new(BounceCounter),
                Box::new(Authorizer::deny_all()),
                CompositeExecutor::new(),
            ),
        );
        // The victim: suspended below so all its inbound is held.
        host.spawn(
            SessionId::new(Hash::of(b"victim")),
            HostedSession::genesis(
                Hash::of(b"receiver-v1"),
                Box::new(ReceiverAgent),
                Box::new(Authorizer::deny_all()),
                CompositeExecutor::new(),
            ),
        );
        host.suspend(&SessionId::new(Hash::of(b"victim"))); // suspend BEFORE running → all victim inbound is held
        let async_host = AsyncAgentHost::new(host);
        let inbox = async_host.inbox();
        let (_sd_tx, sd_rx) = tokio::sync::oneshot::channel();
        // Flood CAP + OVER Emit-shaped inbounds for the suspended victim, each carrying the sender as
        // reply_to (a cross-session Emit) so a shed message bounces.
        for i in 0..(HELD_INBOUND_CAP + OVER) {
            inbox
                .send(Inbound {
                    session: SessionId::new(Hash::of(b"victim")),
                    body: EventBody::Inbound {
                        content_type: ContentType {
                            family: "message".into(),
                            version: 1,
                        },
                        payload: Payload::Inline(vec![i as u8].into()),
                    },
                    cause: None,
                    reply_to: Some(SessionId::new(Hash::of(b"sender"))),
                })
                .unwrap();
        }
        drop(inbox);
        let host = async_host
            .run(sd_rx, || 0)
            .await
            .expect("clean shutdown, no kernel error");

        // The victim is still suspended + never folded anything (all held); the sender folded exactly OVER
        // bounces — the OVER oldest messages shed at the cap, each bounced (not silently lost).
        assert!(host.is_suspended(&SessionId::new(Hash::of(b"victim"))));
        assert_eq!(
            host.get(&SessionId::new(Hash::of(b"sender")))
                .unwrap()
                .session()
                .kv()
                .get(b"bounces")
                .map(|v| v.len()),
            Some(OVER),
            "the OVER oldest held messages shed at the cap each bounced to the sender (bounded, not silent-loss)"
        );
    }

    #[tokio::test]
    async fn lifecycle_suspend_e2e_a_suspended_targets_inbound_is_held_not_folded() {
        // §lifecycle I4 gate (first half): while a target is SUSPENDED its inbound is HELD by the loop — not
        // delivered, not dropped. Drive: controller suspends the victim, THEN a message arrives for the
        // victim (FIFO after the suspend applies) → the loop holds it. No resume → the victim never folds it.
        let mut async_host = AsyncAgentHost::new(AgentHost::new());
        async_host.host_mut().spawn(
            SessionId::new(Hash::of(b"victim")),
            HostedSession::genesis(
                Hash::of(b"receiver-v1"),
                Box::new(ReceiverAgent),
                Box::new(Authorizer::deny_all()),
                CompositeExecutor::new(),
            ),
        );
        let controller = suspend_resume_controller(&async_host, "victim");
        async_host
            .host_mut()
            .spawn(SessionId::new(Hash::of(b"controller")), controller);

        let inbox = async_host.inbox();
        let (_sd_tx, sd_rx) = tokio::sync::oneshot::channel();
        // FIFO: suspend the victim, THEN a message for it (held while suspended). No resume.
        inbox.send(control_trigger(b"suspend")).unwrap();
        inbox.send(message_to("victim", b"held-msg")).unwrap();
        drop(inbox);
        let host = async_host
            .run(sd_rx, || 0)
            .await
            .expect("clean shutdown, no kernel error");

        // The victim is still registered (suspend does not remove it) but NEVER folded the held message —
        // its KV["inbox"] is unset (held, not delivered, not dropped).
        assert!(host.contains(&SessionId::new(Hash::of(b"victim"))));
        assert_eq!(
            host.get(&SessionId::new(Hash::of(b"victim")))
                .unwrap()
                .session()
                .kv()
                .get(b"inbox"),
            None,
            "a suspended target's inbound is HELD, not folded (KV unchanged while suspended)"
        );
    }

    #[tokio::test]
    async fn lifecycle_resume_e2e_held_inbound_replays_on_resume_not_dropped() {
        // §lifecycle I4 gate (second half): a message held during suspension REPLAYS when the target resumes
        // (not dropped). Drive: suspend → message (held) → resume → the loop replays the held inbound and the
        // victim folds it. Same FIFO one-loop-run arc as the terminate E2E.
        let mut async_host = AsyncAgentHost::new(AgentHost::new());
        async_host.host_mut().spawn(
            SessionId::new(Hash::of(b"victim")),
            HostedSession::genesis(
                Hash::of(b"receiver-v1"),
                Box::new(ReceiverAgent),
                Box::new(Authorizer::deny_all()),
                CompositeExecutor::new(),
            ),
        );
        let controller = suspend_resume_controller(&async_host, "victim");
        async_host
            .host_mut()
            .spawn(SessionId::new(Hash::of(b"controller")), controller);

        let inbox = async_host.inbox();
        let (_sd_tx, sd_rx) = tokio::sync::oneshot::channel();
        // FIFO: suspend → message (held) → resume (releases the held message → victim folds it).
        inbox.send(control_trigger(b"suspend")).unwrap();
        inbox.send(message_to("victim", b"resumed-msg")).unwrap();
        inbox.send(control_trigger(b"resume")).unwrap();
        drop(inbox);
        let host = async_host
            .run(sd_rx, || 0)
            .await
            .expect("clean shutdown, no kernel error");

        // The held message replayed on resume and the victim folded it (not dropped): KV["inbox"] now carries
        // the held payload verbatim.
        assert_eq!(
            host.get(&SessionId::new(Hash::of(b"victim")))
                .unwrap()
                .session()
                .kv()
                .get(b"inbox"),
            Some(&b"resumed-msg"[..]),
            "the inbound held during suspension replays on resume + is folded (not dropped)"
        );
        assert!(
            !host.is_suspended(&SessionId::new(Hash::of(b"victim"))),
            "the victim is no longer suspended after resume"
        );
    }

    #[tokio::test]
    async fn lifecycle_resume_e2e_replay_drain_partitions_released_from_still_held() {
        // §lifecycle I4 gate (reviewer #2452 coverage note): the replay-drain PARTITIONS — when a lifecycle op
        // resumes ONE target, its held inbound replays while a DIFFERENT still-suspended target's held inbound
        // STAYS held (not leaked out, not dropped). This is the subtle loop branch (still_held vs deliver) a
        // refactor could silently break. Two victims A + B, both suspended + messaged; resume ONLY A.
        let mut async_host = AsyncAgentHost::new(AgentHost::new());
        for v in ["victim-a", "victim-b"] {
            async_host.host_mut().spawn(
                SessionId::new(Hash::of(v.as_bytes())),
                HostedSession::genesis(
                    Hash::of(b"receiver-v1"),
                    Box::new(ReceiverAgent),
                    Box::new(Authorizer::deny_all()),
                    CompositeExecutor::new(),
                ),
            );
        }
        // Two controllers: one suspends+resumes A, one suspends B (B never resumes). Build both BEFORE the
        // host_mut() spawn — they borrow `&async_host` (for the lifecycle channel), which can't overlap the
        // `&mut` from host_mut().
        let controller_a = suspend_resume_controller(&async_host, "victim-a");
        let victim_b_hex = SessionId::new(Hash::of(b"victim-b")).to_hex();
        let controller_b = HostedSession::genesis(
            Hash::of(b"suspend-b-controller-v1"),
            Box::new(SuspendResumeController {
                target: victim_b_hex.clone(),
            }),
            Box::new(
                Authorizer::new(vec![]).with_family_grants(vec![Capability::for_family(
                    effect_ct::LIFECYCLE_SUSPEND,
                    ResourcePredicate::Exact(victim_b_hex.into()),
                )]),
            ),
            CompositeExecutor::new().with_effect(
                effect_ct::LIFECYCLE_SUSPEND,
                Box::new(crate::LifecycleExecutor::new(
                    async_host.lifecycle_channel(),
                    SessionId::new(Hash::of(b"controller-b")),
                    Hash::of(b"controller-b"),
                )),
            ),
        );
        async_host
            .host_mut()
            .spawn(SessionId::new(Hash::of(b"controller")), controller_a);
        async_host
            .host_mut()
            .spawn(SessionId::new(Hash::of(b"controller-b")), controller_b);

        let inbox = async_host.inbox();
        let (_sd_tx, sd_rx) = tokio::sync::oneshot::channel();
        // FIFO: suspend A, suspend B, message both (both held), resume ONLY A.
        inbox.send(control_trigger(b"suspend")).unwrap(); // controller → suspend victim-a
        inbox
            .send(Inbound {
                session: SessionId::new(Hash::of(b"controller-b")),
                body: EventBody::Inbound {
                    content_type: ContentType {
                        family: "message".into(),
                        version: 1,
                    },
                    payload: Payload::Inline(b"suspend".to_vec().into()),
                },
                cause: None,
                reply_to: None,
            })
            .unwrap(); // controller-b → suspend victim-b
        inbox.send(message_to("victim-a", b"msg-a")).unwrap();
        inbox.send(message_to("victim-b", b"msg-b")).unwrap();
        inbox.send(control_trigger(b"resume")).unwrap(); // controller → resume victim-a ONLY
        drop(inbox);
        let host = async_host
            .run(sd_rx, || 0)
            .await
            .expect("clean shutdown, no kernel error");

        // A resumed → its held inbound replayed + folded.
        assert_eq!(
            host.get(&SessionId::new(Hash::of(b"victim-a")))
                .unwrap()
                .session()
                .kv()
                .get(b"inbox"),
            Some(&b"msg-a"[..]),
            "resumed target A's held inbound replays + folds (partition RELEASED it)"
        );
        // B still suspended → its held inbound stays held (NOT leaked to it, NOT dropped): KV unset + still suspended.
        assert!(
            host.is_suspended(&SessionId::new(Hash::of(b"victim-b"))),
            "B is still suspended (never resumed)"
        );
        assert_eq!(
            host.get(&SessionId::new(Hash::of(b"victim-b")))
                .unwrap()
                .session()
                .kv()
                .get(b"inbox"),
            None,
            "still-suspended target B's held inbound stays held (partition KEPT it — not leaked, not dropped)"
        );
    }

    #[tokio::test]
    async fn lifecycle_held_emit_to_a_target_terminated_while_suspended_bounces_to_the_sender() {
        // §lifecycle I4/I5 (#2452 Copilot c1 fix): a cross-session Emit (reply_to set) HELD while its target
        // is suspended, whose target then TERMINATES before resume, must BOUNCE a delivery-failure to the
        // emitter on the replay-drain — NOT drop silently (a held Emit retains its live reply_to). Drive:
        // suspend victim → a held Emit for victim (reply_to=sender) → terminate victim → resume victim →
        // replay-drain finds victim gone → bounces to sender, who folds it.
        let mut async_host = AsyncAgentHost::new(AgentHost::new());
        // The SENDER (a live registered session) records any delivery-failure bounce it receives.
        async_host.host_mut().spawn(
            SessionId::new(Hash::of(b"sender")),
            HostedSession::genesis(
                Hash::of(b"bounce-recorder-v1"),
                Box::new(BounceRecorder),
                Box::new(Authorizer::deny_all()),
                CompositeExecutor::new(),
            ),
        );
        // The VICTIM: an ordinary session, suspended then terminated below.
        async_host.host_mut().spawn(
            SessionId::new(Hash::of(b"victim")),
            HostedSession::genesis(
                Hash::of(b"receiver-v1"),
                Box::new(ReceiverAgent),
                Box::new(Authorizer::deny_all()),
                CompositeExecutor::new(),
            ),
        );
        // A controller that suspends victim (b"suspend") / resumes victim (b"resume").
        let sr_controller = suspend_resume_controller(&async_host, "victim");
        async_host
            .host_mut()
            .spawn(SessionId::new(Hash::of(b"controller")), sr_controller);
        // A controller that terminates victim (on b"go"), addressing it by its canonical genesis-hash hex.
        let victim_hex = SessionId::new(Hash::of(b"victim")).to_hex();
        let killer = HostedSession::genesis(
            Hash::of(b"killer-v1"),
            Box::new(TerminatorAgent {
                victim: victim_hex.clone(),
            }),
            Box::new(
                Authorizer::new(vec![]).with_family_grants(vec![Capability::for_family(
                    effect_ct::LIFECYCLE_TERMINATE,
                    ResourcePredicate::Exact(victim_hex.into()),
                )]),
            ),
            CompositeExecutor::new().with_effect(
                effect_ct::LIFECYCLE_TERMINATE,
                Box::new(crate::LifecycleExecutor::new(
                    async_host.lifecycle_channel(),
                    SessionId::new(Hash::of(b"killer")),
                    Hash::of(b"killer"),
                )),
            ),
        );
        async_host
            .host_mut()
            .spawn(SessionId::new(Hash::of(b"killer")), killer);

        let inbox = async_host.inbox();
        let (_sd_tx, sd_rx) = tokio::sync::oneshot::channel();
        // FIFO: suspend victim → a HELD cross-session Emit for victim (reply_to=sender) → terminate victim
        // (marks + removes it while its inbound is held) → resume victim → replay-drain finds victim gone →
        // bounce to sender.
        inbox.send(control_trigger(b"suspend")).unwrap();
        inbox
            .send(Inbound {
                session: SessionId::new(Hash::of(b"victim")),
                body: EventBody::Inbound {
                    content_type: ContentType {
                        family: "message".into(),
                        version: 1,
                    },
                    payload: Payload::Inline(b"held-emit".to_vec().into()),
                },
                cause: None,
                reply_to: Some(SessionId::new(Hash::of(b"sender"))), // a cross-session Emit's return-address
            })
            .unwrap();
        inbox
            .send(Inbound {
                session: SessionId::new(Hash::of(b"killer")),
                body: go(),
                cause: None,
                reply_to: None,
            })
            .unwrap();
        inbox.send(control_trigger(b"resume")).unwrap();
        drop(inbox);
        let host = async_host
            .run(sd_rx, || 0)
            .await
            .expect("clean shutdown, no kernel error");

        // The victim was terminated + removed.
        assert!(
            !host.contains(&SessionId::new(Hash::of(b"victim"))),
            "victim terminated while suspended + removed"
        );
        // The sender folded a delivery-failure bounce carrying the held Emit's echoed payload — the held
        // Emit did NOT drop silently (the #2452 c1 fix).
        assert_eq!(
            host.get(&SessionId::new(Hash::of(b"sender")))
                .unwrap()
                .session()
                .kv()
                .get(b"bounced"),
            Some(&b"held-emit"[..]),
            "a held Emit to a target terminated-while-suspended BOUNCES to the sender (not dropped)"
        );
    }

    /// A timer agent: arms a timer at `deadline_ms` on "go"; records "woke" when it fires (PR#1303 fix).
    struct TimerAgent {
        deadline_ms: u64,
    }
    #[async_trait::async_trait(?Send)]
    impl Reducer for TimerAgent {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    FoldOutput::with(vec![EffectRequest::new_with_family(
                        effect_ct::TIMER,
                        self.deadline_ms.to_string(),
                        None,
                        Timeliness::Interactive,
                    )])
                }
                EventBody::TimerFired { .. } => {
                    kv.put(b"woke".to_vec(), b"1".to_vec());
                    FoldOutput::none()
                }
                _ => FoldOutput::none(),
            }
        }
    }

    #[tokio::test]
    async fn an_already_due_timer_fires_before_the_loop_blocks_on_the_inbox() {
        // PR#1303 starvation fix: the loop fires due timers UP FRONT (before select!), so a timer whose
        // deadline has already passed at loop entry fires even though the inbox has a queued message the
        // loop would otherwise process first. Arm a timer for t=1000, deliver "go" (arms it) via the
        // registry before running, then run with the clock already at 5000 (past the deadline) → the
        // up-front fire wakes it. Shutdown after so the loop returns.
        let mut host = AgentHost::new();
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Timer,
            predicate: ResourcePredicate::Any,
        }]);
        host.spawn(
            SessionId::new(Hash::of(b"t")),
            HostedSession::genesis(
                Hash::of(b"timer-v1"),
                Box::new(TimerAgent { deadline_ms: 1000 }),
                Box::new(authz),
                CompositeExecutor::new(),
            ),
        );
        // Arm the timer directly (pre-run) so it's already armed when the loop starts.
        host.deliver(&SessionId::new(Hash::of(b"t")), go(), None)
            .await;
        assert_eq!(
            host.get(&SessionId::new(Hash::of(b"t")))
                .unwrap()
                .session()
                .kv()
                .get(b"woke"),
            None,
            "not fired yet"
        );

        let async_host = AsyncAgentHost::new(host);
        let (sd_tx, sd_rx) = tokio::sync::oneshot::channel();
        // Shut down immediately; the clock is already past the deadline, so the up-front fire runs before
        // the shutdown arm is taken.
        sd_tx.send(()).unwrap();
        let host = async_host
            .run(sd_rx, || 5000)
            .await
            .expect("clean shutdown");
        assert_eq!(
            host.get(&SessionId::new(Hash::of(b"t")))
                .unwrap()
                .session()
                .kv()
                .get(b"woke"),
            Some(&b"1"[..]),
            "an already-due timer fires up front, not starved by shutdown/inbox"
        );
    }

    #[tokio::test]
    async fn run_with_wall_clock_drives_a_session_end_to_end() {
        // The real-process convenience: run the loop on the SYSTEM clock (not an injected closure) and prove
        // it still drives a session's turn to completion. No timers armed here, so the wall clock only feeds
        // the (never-taken) sleep arm — the point is that run_with_wall_clock wires the clock + delegates to
        // run() correctly, so a deployed daemon can call it with no clock plumbing.
        let mut host = AgentHost::new();
        host.spawn(SessionId::new(Hash::of(b"wall")), mark_host());
        let async_host = AsyncAgentHost::new(host);
        let inbox = async_host.inbox();
        let (_sd_tx, sd_rx) = tokio::sync::oneshot::channel();
        inbox
            .send(Inbound {
                session: SessionId::new(Hash::of(b"wall")),
                body: go(),
                cause: None,
                reply_to: None,
            })
            .unwrap();
        drop(inbox); // channel closes after the one message → loop returns

        let host = async_host
            .run_with_wall_clock(sd_rx)
            .await
            .expect("clean shutdown on the wall clock, no kernel error");
        assert_eq!(
            host.get(&SessionId::new(Hash::of(b"wall")))
                .unwrap()
                .session()
                .kv()
                .get(b"ran"),
            Some(&b"1"[..]),
            "the session ran its turn through the wall-clock loop"
        );
    }

    // ── admin control interface, wired into the loop ────────────────────────────────────────────────

    use crate::admin::{AdminCommand, AdminResponse, InstallSpec, SessionFactory};

    /// A factory that builds a canned mark-agent session for any spec — the test stand-in for the real
    /// wasm-loading factory, so an `install-session` command through the loop actually registers a session.
    struct StubFactory;
    #[async_trait::async_trait(?Send)]
    impl SessionFactory for StubFactory {
        async fn build(&mut self, _spec: &InstallSpec) -> Result<HostedSession, String> {
            Ok(mark_host())
        }
        // Spawn support: build a live child session with parent-provenance + the caller's nonce (a ReceiverAgent
        // so a test can drive it), mirroring the real ComponentSessionFactory::build_spawned shape.
        async fn build_spawned(
            &mut self,
            reducer_hash: Hash,
            parent_genesis: Hash,
            spawn_nonce: Hash,
        ) -> Result<HostedSession, String> {
            Ok(HostedSession::genesis_spawned_with_nonce(
                reducer_hash,
                spawn_nonce,
                parent_genesis,
                Box::new(ReceiverAgent),
                Box::new(Authorizer::deny_all()),
                CompositeExecutor::new(),
            ))
        }
    }

    fn install(id: &str) -> AdminCommand {
        AdminCommand::InstallSession(InstallSpec {
            id: SessionId::new(Hash::of(id.as_bytes())),
            reducer_hash: Hash::of(id.as_bytes()),
            goal: None,
        })
    }

    /// Submit one admin command through the channel and await the reply — the request/reply round-trip a
    /// socket listener performs per frame. Asserts the `"admin"` principal (the tests grant it via
    /// `with_admin_authz`).
    async fn admin_call(ch: &AdminChannel, command: AdminCommand) -> AdminResponse {
        let (reply, rx) = tokio::sync::oneshot::channel();
        ch.send(AdminRequest {
            command,
            principal: Some("admin".to_string()),
            reply,
        })
        .unwrap();
        rx.await.expect("the loop replied")
    }

    /// The test authorizer: grants ONLY the `"admin"` principal (the one `admin_call` asserts) every v0
    /// action. Scoped to that specific principal — NOT a `"*"` wildcard — so the loop tests actually
    /// exercise the principal PLUMBING: a request arriving with a wrong/absent principal is DENIED, which is
    /// the security half of the authz wiring (#1975 review). A host built without an authorizer denies all.
    fn test_authz() -> Box<dyn AdminAuthorizer> {
        Box::new(
            AllowList::deny_all()
                .allow("admin", "admin/install-session")
                .allow("admin", "admin/list-sessions")
                .allow("admin", "admin/session-status")
                .allow("admin", "admin/stop-session"),
        )
    }

    #[tokio::test]
    async fn admin_install_then_list_through_the_loop() {
        // The control interface end-to-end (in-process): submit install-session commands over the admin
        // channel, the loop applies each via apply_admin on the loop task + replies, and a subsequent
        // list-sessions reflects them. `AsyncAgentHost` is !Send (owns the factory + registry), so we drive
        // the loop + the client CONCURRENTLY on ONE task via `join!` (no spawn / no Send) — exactly the
        // single-threaded shape the daemon runs.
        let async_host = AsyncAgentHost::with_factory(AgentHost::new(), Box::new(StubFactory))
            .with_admin_authz(test_authz());
        let admin = async_host.admin_channel();
        let (_sd_tx, sd_rx) = tokio::sync::oneshot::channel();

        let client = async move {
            // Install two sessions, then list.
            assert_eq!(
                admin_call(&admin, install("a")).await,
                AdminResponse::Installed {
                    id: SessionId::new(Hash::of(b"a"))
                }
            );
            assert_eq!(
                admin_call(&admin, install("b")).await,
                AdminResponse::Installed {
                    id: SessionId::new(Hash::of(b"b"))
                }
            );
            let mut expected_ids = vec![
                SessionId::new(Hash::of(b"a")),
                SessionId::new(Hash::of(b"b")),
            ];
            expected_ids.sort(); // listed sorted by SessionId (= genesis-hash byte order)
            assert_eq!(
                admin_call(&admin, AdminCommand::ListSessions).await,
                AdminResponse::Sessions { ids: expected_ids }
            );
            // A stop, then list reflects the removal.
            assert_eq!(
                admin_call(
                    &admin,
                    AdminCommand::StopSession {
                        id: SessionId::new(Hash::of(b"a"))
                    }
                )
                .await,
                AdminResponse::Stopped {
                    id: SessionId::new(Hash::of(b"a"))
                }
            );
            assert_eq!(
                admin_call(&admin, AdminCommand::ListSessions).await,
                AdminResponse::Sessions {
                    ids: vec![SessionId::new(Hash::of(b"b"))]
                }
            );
            // Drop the admin channel (last producer) → the loop's channels close → run() returns.
            drop(admin);
        };

        let (loop_result, ()) = tokio::join!(async_host.run(sd_rx, || 0), client);
        loop_result.expect("clean shutdown, no kernel error");
    }

    #[tokio::test]
    async fn admin_install_without_a_factory_errors_but_the_loop_keeps_serving() {
        // A host built with `new` (no factory): an install-session is a clean error over the channel, and
        // the loop stays alive to serve a following list (a bad admin command never wedges the daemon).
        let async_host = AsyncAgentHost::new(AgentHost::new()).with_admin_authz(test_authz());
        let admin = async_host.admin_channel();
        let (_sd_tx, sd_rx) = tokio::sync::oneshot::channel();

        let client = async move {
            let resp = admin_call(&admin, install("x")).await;
            assert!(
                matches!(&resp, AdminResponse::Error { message } if message.contains("no session factory")),
                "install with no factory is a clean error: {resp:?}"
            );
            // The loop is still serving — a list works and shows nothing was installed.
            assert_eq!(
                admin_call(&admin, AdminCommand::ListSessions).await,
                AdminResponse::Sessions { ids: vec![] }
            );
            drop(admin);
        };

        let (loop_result, ()) = tokio::join!(async_host.run(sd_rx, || 0), client);
        loop_result.expect("clean shutdown");
    }

    #[tokio::test]
    async fn shutdown_ends_the_loop_even_with_a_live_admin_channel() {
        // A live admin channel (like a live inbox) must not keep the loop from shutting down — firing
        // shutdown ends run() promptly even though an admin sender is still held.
        let async_host = AsyncAgentHost::with_factory(AgentHost::new(), Box::new(StubFactory))
            .with_admin_authz(test_authz());
        let _admin = async_host.admin_channel(); // held alive → admin channel stays open
        let (sd_tx, sd_rx) = tokio::sync::oneshot::channel();
        sd_tx.send(()).unwrap();
        // Should return via the shutdown arm, not hang on the still-open admin/inbox channels.
        async_host
            .run(sd_rx, || 0)
            .await
            .expect("shutdown returns Ok");
    }

    #[tokio::test]
    async fn the_loop_enforces_admin_authz_deny_by_default() {
        // The wiring this slice adds: the loop gates every admin command through its authorizer. A host
        // built WITHOUT a configured authorizer (default deny-all) refuses commands — even a list — and
        // NEVER touches the registry. Proves the loop calls apply_admin_authorized, not the bare applier.
        let async_host = AsyncAgentHost::with_factory(AgentHost::new(), Box::new(StubFactory));
        // ^ no .with_admin_authz → deny-all default
        let admin = async_host.admin_channel();
        let (_sd_tx, sd_rx) = tokio::sync::oneshot::channel();

        let client = async move {
            // An install is denied (deny-all), registry untouched.
            let resp = admin_call(&admin, install("blocked")).await;
            assert!(
                matches!(&resp, AdminResponse::Error { message } if message.contains("denied")),
                "deny-all host refuses the install: {resp:?}"
            );
            // Even a read (list) is denied by the deny-all default.
            let listed = admin_call(&admin, AdminCommand::ListSessions).await;
            assert!(
                matches!(&listed, AdminResponse::Error { message } if message.contains("denied")),
                "deny-all refuses list too: {listed:?}"
            );
            drop(admin);
        };

        let (loop_result, ()) = tokio::join!(async_host.run(sd_rx, || 0), client);
        let host = loop_result.expect("clean shutdown");
        assert!(host.is_empty(), "a denied install left the registry empty");
    }

    #[tokio::test]
    async fn an_anonymous_principal_is_denied_even_when_the_named_admin_is_allowed() {
        // Principal plumbing: the authorizer grants "admin" but a request with NO principal (None → "") is
        // anonymous, so a deny-by-default AllowList (which only granted "admin") refuses it.
        let authz: Box<dyn AdminAuthorizer> =
            Box::new(AllowList::deny_all().allow("admin", "admin/list-sessions"));
        let async_host = AsyncAgentHost::new(AgentHost::new()).with_admin_authz(authz);
        let admin = async_host.admin_channel();
        let (_sd_tx, sd_rx) = tokio::sync::oneshot::channel();

        let client = async move {
            // Anonymous (principal None) → denied.
            let (reply, rx) = tokio::sync::oneshot::channel();
            admin
                .send(AdminRequest {
                    command: AdminCommand::ListSessions,
                    principal: None,
                    reply,
                })
                .unwrap();
            let anon = rx.await.unwrap();
            assert!(
                matches!(&anon, AdminResponse::Error { message } if message.contains("denied")),
                "anonymous principal is denied: {anon:?}"
            );
            // The named "admin" principal IS allowed for list.
            let named = admin_call(&admin, AdminCommand::ListSessions).await;
            assert_eq!(named, AdminResponse::Sessions { ids: vec![] });
            drop(admin);
        };

        let (loop_result, ()) = tokio::join!(async_host.run(sd_rx, || 0), client);
        loop_result.expect("clean shutdown");
    }

    // ---- userspace-effects I4 loop-side: the reply-settle seam ----

    /// `AgentHost::settle_reply` reports `false` for an absent caller (the registry-facing no-op the loop's
    /// reply-settle drain relies on), distinct from a landed settle — the belt to the I6 terminate-prune's
    /// token refusal, and the reason a late/stale `effect/reply` to a gone caller can't corrupt a log.
    #[tokio::test]
    async fn agent_host_settle_reply_is_false_for_an_absent_caller() {
        use cdz_kernel::effect::EffectId;
        let mut host = AgentHost::new();
        let landed = host
            .settle_reply(
                &SessionId::new(Hash::of(b"never-registered")),
                EffectId(7),
                EffectOutcome::Ok(None),
            )
            .await;
        assert!(
            !landed,
            "settling an absent caller lands nothing (benign false)"
        );
    }

    // ---- the SELF-HOSTING TICK-LOOP end-to-end (converted from the deleted self_hosting_tick_loop_e2e
    // integration test, operator no-integration-tests mandate — hermetic: an agent-as-reducer + the real
    // AsyncAgentHost loop, no wasm/network). GAP-5, the self-hosting-harness endgame: a session RE-ARMS its
    // own timer each tick so the loop re-drives it — the SAME "wake me again next interval" pattern fleet.rs
    // does via a cron re-issue, but in userspace-on-the-harness (a reducer + the existing timer/loop
    // mechanism), NO new host machinery. The host provides MECHANISM (timer effect + timer wheel); the tick
    // cadence/work/stop are the reducer's policy. ----

    /// Ticks the role runs before it stops re-arming (its self-imposed budget — a real role loops until a
    /// stop signal; a bounded budget keeps the test deterministic + terminating).
    const TICK_BUDGET: u64 = 5;
    /// The tick interval the role arms (ms). Irrelevant to the test — `now_ms` is driven far past it so each
    /// re-armed timer is immediately due, cycling the loop fast to quiescence.
    const TICK_INTERVAL_MS: u64 = 1000;

    /// A self-re-arming TICK-LOOP role reducer (the fleet.rs re-issue shape, in userspace). Stores its tick
    /// count in KV (durable — a recovered session resumes its loop). Stops re-arming at the budget so the
    /// loop drains.
    struct TickLoopRole;
    impl TickLoopRole {
        fn arm_tick() -> EffectRequest {
            EffectRequest::new_with_family(
                effect_ct::TIMER,
                TICK_INTERVAL_MS.to_string(),
                None,
                Timeliness::Interactive,
            )
        }
        fn ticks(kv: &Kv) -> u64 {
            kv.get(b"ticks")
                .and_then(|b| std::str::from_utf8(b).ok())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0)
        }
    }
    #[async_trait::async_trait(?Send)]
    impl Reducer for TickLoopRole {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                // The kick: start the loop by arming the first tick.
                EventBody::Inbound { .. } => FoldOutput::with(vec![Self::arm_tick()]),
                // Each tick fires: do the tick's work (bump the counter) + re-arm for the next — until budget.
                EventBody::TimerFired { .. } => {
                    let n = Self::ticks(kv) + 1;
                    kv.put(b"ticks".to_vec(), n.to_string().into_bytes());
                    if n < TICK_BUDGET {
                        // Re-arm: the loop's timer wheel fires the next tick → this fold runs again. THE
                        // self-perpetuating loop — the reducer decides to continue (policy), the host fires.
                        FoldOutput::with(vec![Self::arm_tick()])
                    } else {
                        // Budget reached → stop re-arming. With no armed timer + a closed inbox, the loop drains.
                        kv.put(b"done".to_vec(), b"1".to_vec());
                        FoldOutput::none()
                    }
                }
                _ => FoldOutput::none(),
            }
        }
    }

    fn tick_kick() -> EventBody {
        EventBody::Inbound {
            content_type: ContentType {
                family: "message".into(),
                version: 1,
            },
            payload: Payload::Inline(b"start-your-loop".to_vec().into()),
        }
    }

    #[tokio::test]
    async fn agent_reducer_runs_a_self_rearming_tick_loop_through_the_host_loop() {
        // Grant only `timer` (the role re-arms it each tick); deny-by-default otherwise.
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Timer,
            predicate: ResourcePredicate::Any,
        }]);
        let mut host = AgentHost::new();
        let id = SessionId::new(Hash::of(b"tick-role"));
        host.spawn(
            id,
            HostedSession::genesis(
                Hash::of(b"tick-role-v1"),
                Box::new(TickLoopRole),
                Box::new(authz),
                CompositeExecutor::new(),
            ),
        );
        // Kick the loop BEFORE running so the first timer is armed at loop entry.
        host.deliver(&id, tick_kick(), None).await;
        assert_eq!(
            TickLoopRole::ticks(host.get(&id).unwrap().session().kv()),
            0,
            "no ticks yet — the timer is armed but hasn't fired"
        );

        // Run the loop with the clock pinned FAR PAST any deadline, so each re-armed timer is immediately due:
        // the loop fires the tick → the role re-arms → fires again → … until the role stops re-arming at the
        // budget, at which point (no armed timer, inbox closed) the loop drains and `run` returns. No shutdown
        // signal needed — the self-stopping role IS the termination (the fleet-loop's stop condition).
        let async_host = AsyncAgentHost::new(host);
        let (_sd_tx, sd_rx) = oneshot::channel();
        let host = async_host
            .run(sd_rx, || u64::MAX)
            .await
            .expect("the loop drains cleanly once the role stops re-arming");

        // The role ran exactly TICK_BUDGET ticks, then stopped — a self-hosted tick-loop, driven to completion
        // by the host's timer wheel with no external re-issue (the fleet.rs cron replaced by the re-arm).
        let kv = host.get(&id).unwrap();
        let kv = kv.session().kv();
        assert_eq!(
            TickLoopRole::ticks(kv),
            TICK_BUDGET,
            "the role ran exactly its tick budget, re-arming each tick through the host loop"
        );
        assert_eq!(
            kv.get(b"done").map(|v| v.to_vec()),
            Some(b"1".to_vec()),
            "the role reached its stop condition + stopped re-arming (the loop then drained)"
        );
    }

    // ---- real-daemon BOOT + admin-INSTALL + live-TICK smoke (converted from the deleted
    // daemon_boot_install_tick_smoke integration test, operator no-integration-tests mandate — hermetic:
    // native TickRole, no wasm/network). Beyond the direct-loop tick test above: exercises the DEPLOYED chain
    // — boot AsyncAgentHost as a control plane + a SessionFactory install seam + admin channel, INSTALL a
    // session at runtime through the admin channel (exactly what the socket listener does per frame), and let
    // the running loop drive it live to its tick budget. ----
    const BOOT_TICK_BUDGET: u64 = 3;

    /// The installed agent: a self-re-arming tick-loop role (the fleet.rs re-issue shape). Timer-only →
    /// hermetic. (Distinct from `TickLoopRole` above — this one is installed via the factory + admin path.)
    struct TickRole;
    impl TickRole {
        fn arm() -> EffectRequest {
            EffectRequest::new_with_family(effect_ct::TIMER, "1000", None, Timeliness::Interactive)
        }
        fn ticks(kv: &Kv) -> u64 {
            kv.get(b"ticks")
                .and_then(|b| std::str::from_utf8(b).ok())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0)
        }
    }
    #[async_trait::async_trait(?Send)]
    impl Reducer for TickRole {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => FoldOutput::with(vec![Self::arm()]),
                EventBody::TimerFired { .. } => {
                    let n = Self::ticks(kv) + 1;
                    kv.put(b"ticks".to_vec(), n.to_string().into_bytes());
                    if n < BOOT_TICK_BUDGET {
                        FoldOutput::with(vec![Self::arm()])
                    } else {
                        kv.put(b"done".to_vec(), b"1".to_vec());
                        FoldOutput::none()
                    }
                }
                _ => FoldOutput::none(),
            }
        }
    }

    /// A minimal [`SessionFactory`] that installs the [`TickRole`] with a `timer` grant — standing in for the
    /// deployed [`ComponentSessionFactory`](crate::ComponentSessionFactory) (which loads a wasm reducer from a
    /// blob store). The boot→admin-install→run chain is identical; only the reducer SOURCE differs.
    struct TickRoleFactory;
    #[async_trait::async_trait(?Send)]
    impl SessionFactory for TickRoleFactory {
        async fn build(&mut self, spec: &InstallSpec) -> Result<HostedSession, String> {
            let authz = Authorizer::new(vec![Capability {
                kind: EffectKind::Timer,
                predicate: ResourcePredicate::Any,
            }]);
            Ok(HostedSession::genesis(
                spec.reducer_hash,
                Box::new(TickRole),
                Box::new(authz),
                CompositeExecutor::new(),
            ))
        }
        async fn build_spawned(
            &mut self,
            _reducer_hash: Hash,
            _parent_genesis: Hash,
            _spawn_nonce: Hash,
        ) -> Result<HostedSession, String> {
            Err("TickRoleFactory does not spawn children".to_string())
        }
    }

    #[tokio::test]
    async fn daemon_boots_admin_installs_a_role_and_it_ticks_live_through_the_loop() {
        // The host loop + its sessions are `!Send` (single-threaded, §15b), so drive inside a LocalSet where
        // `spawn_local` is valid — the same shape the deployed daemon's current_thread runtime + LocalSet give.
        tokio::task::LocalSet::new()
            .run_until(async {
                // BOOT: control-plane shape — empty registry + a factory (install seam) + admin authorizer
                // granting the local admin.
                let async_host =
                    AsyncAgentHost::with_factory(AgentHost::new(), Box::new(TickRoleFactory))
                        .with_admin_authz(Box::new(AllowList::allow_all_for_local_admin()));
                let admin = async_host.admin_channel();
                let inbox = async_host.inbox();

                // Spawn the loop, clock pinned past every deadline so live ticks fire promptly.
                let (_sd_tx, sd_rx) = oneshot::channel();
                let loop_task =
                    tokio::task::spawn_local(async move { async_host.run(sd_rx, || u64::MAX).await });

                // INSTALL a session at runtime through the admin channel (what the socket listener does per frame).
                let id = SessionId::new(Hash::of(b"worker-1"));
                let resp = admin_call(
                    &admin,
                    AdminCommand::InstallSession(InstallSpec {
                        id,
                        reducer_hash: Hash::of(b"tick-role-v1"),
                        goal: Some("run your tick loop".to_string()),
                    }),
                )
                .await;
                assert!(
                    matches!(&resp, AdminResponse::Installed { id: got } if *got == id),
                    "the daemon installed the session, got {resp:?}"
                );

                // KICK the installed agent's loop via the inbox; the timer wheel then fires it to its budget.
                inbox
                    .send(Inbound {
                        session: id,
                        body: EventBody::Inbound {
                            content_type: ContentType {
                                family: "message".into(),
                                version: 1,
                            },
                            payload: Payload::Inline(b"go".to_vec().into()),
                        },
                        cause: None,
                        reply_to: None,
                    })
                    .expect("inbox accepts the kick");

                // Drop the producer handles so the loop drains once the role stops re-arming (budget reached).
                drop(admin);
                drop(inbox);

                let host = loop_task
                    .await
                    .expect("loop task joined")
                    .expect("loop drained cleanly");

                // The installed agent TICKED LIVE through the deployed loop, to its budget.
                let session = host
                    .get(&id)
                    .expect("worker still registered");
                let kv = session.session().kv();
                assert_eq!(
                    TickRole::ticks(kv),
                    BOOT_TICK_BUDGET,
                    "the admin-installed agent ran its full tick budget live through the daemon loop"
                );
                assert_eq!(
                    kv.get(b"done").map(|v| v.to_vec()),
                    Some(b"1".to_vec()),
                    "the live agent reached its stop condition"
                );
            })
            .await;
    }

    // ---- real-daemon BOOT + admin-install-a-WASM-REDUCER-by-hash + live-run smoke (converted from the
    // deleted daemon_boot_wasm_reducer_smoke, operator no-integration-tests mandate). ENV-GATED on
    // CDZ_LIVE_REDUCER_COMPONENT (a real lifted wasm reducer the nix build produces): unset → SKIP cleanly.
    // Closes the last gap over the native-factory smoke above: the REAL ComponentSessionFactory blob-fetches
    // the bytes by content hash + LIFTS them into an AsyncComponentReducer, and the loop drives that wasm
    // program live to quiescence — the deployed reducer-load path end to end. ----
    #[tokio::test]
    async fn daemon_boots_admin_installs_a_wasm_reducer_by_hash_and_it_runs_live() {
        let Some(component) = crate::test_support::reducer_component_bytes() else {
            eprintln!(
                "SKIP daemon_boots_admin_installs_a_wasm_reducer_by_hash_and_it_runs_live: \
                 CDZ_LIVE_REDUCER_COMPONENT unset — set it to a real wasm reducer component (the nix build \
                 produces one) to exercise the boot→admin-install→blob-lift→run path."
            );
            return;
        };
        tokio::task::LocalSet::new()
            .run_until(wasm_reducer_smoke(component))
            .await;
    }

    async fn wasm_reducer_smoke(component: Vec<u8>) {
        use crate::ComponentSessionFactory;
        use cdz_kernel::blob::BlobStore;
        use std::time::Duration;

        // A hard ceiling on the whole boot→install→run→drain arc: a misbehaving/looping wasm reducer could
        // otherwise hang the suite — bounding the join surfaces a runaway as a clear timeout, not a wedge.
        const RUN_TIMEOUT: Duration = Duration::from_secs(30);

        // The reducer blob store the deployed daemon's install factory loads from. Put the real wasm component
        // in it → content-addressed. Compute the hash ONCE + supply it to put (the post-blob-dual-land sig).
        let mut blob = cdz_kernel::blob::MemBlobStore::new();
        let reducer_hash = Hash::of(&component);
        blob.put(reducer_hash, bytes::Bytes::from(component.clone()))
            .await
            .expect("put the wasm reducer component into the blob store");

        // BOOT the REAL ComponentSessionFactory (the deployed reducer-load seam) — network-free: an empty
        // per-session executor set + a deny-all per-session authorizer.
        let factory = ComponentSessionFactory::new(
            blob,
            CompositeExecutor::new,
            || -> Box<dyn cdz_kernel::authz::Authorize> { Box::new(Authorizer::deny_all()) },
        );

        let async_host = AsyncAgentHost::with_factory(AgentHost::new(), Box::new(factory))
            .with_admin_authz(Box::new(AllowList::allow_all_for_local_admin()));
        let admin = async_host.admin_channel();
        let inbox = async_host.inbox();

        let (sd_tx, sd_rx) = oneshot::channel();
        let loop_task =
            tokio::task::spawn_local(async move { async_host.run(sd_rx, || u64::MAX).await });

        // INSTALL a session BY CONTENT HASH through the admin channel — the factory blob-fetches + LIFTS the
        // bytes into a runnable AsyncComponentReducer (a missing blob or a non-lifting component fails loud).
        let id = SessionId::new(Hash::of(b"wasm-worker-1"));
        let resp = admin_call(
            &admin,
            AdminCommand::InstallSession(InstallSpec {
                id,
                reducer_hash,
                goal: Some("run the compiled reducer".to_string()),
            }),
        )
        .await;
        assert!(
            matches!(&resp, AdminResponse::Installed { id: got } if *got == id),
            "the daemon lifted the wasm reducer from the blob store + installed the session, got {resp:?}"
        );

        // KICK it: the lifted wasm reducer folds one live turn — whatever it emits is denied/declined by the
        // deny-all authorizer + empty executor set; the point is the LIFTED program RUNS through the loop (§17).
        inbox
            .send(Inbound {
                session: id,
                body: EventBody::Inbound {
                    content_type: ContentType {
                        family: "message".into(),
                        version: 1,
                    },
                    payload: Payload::Inline(b"go".to_vec().into()),
                },
                cause: None,
                reply_to: None,
            })
            .expect("inbox accepts the kick");

        // Signal shutdown + drop the producers so the loop drains after the kick's turn settles.
        let _ = sd_tx.send(());
        drop(admin);
        drop(inbox);

        tokio::time::timeout(RUN_TIMEOUT, loop_task)
            .await
            .expect("the boot→install→run→drain arc completes within RUN_TIMEOUT (not a runaway)")
            .expect("loop task joined")
            .expect("loop drained cleanly (the wasm reducer's kick turn ran to quiescence)");
    }
}
