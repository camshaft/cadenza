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
    // SessionIds (genesis-hash hex, host-authored identity) and `reason` is a host-authored delivery-failure
    // cause (absent-target / FoldRefused) — none is guest-controlled payload, so no guest-string-logging
    // concern (§4 keeps the echoed payload out of the log entirely).
    tracing::warn!(
        sender = %sender.as_str(),
        failed_target = %failed_target.as_str(),
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
) -> Result<(), KernelError> {
    while let Ok(op) = lifecycle_rx.try_recv() {
        match op {
            LifecycleOp::Terminate { target, by, reason } => {
                let by_hash =
                    Hash::from_hex(by.as_str()).unwrap_or_else(|| Hash::of(by.as_str().as_bytes()));
                match host.terminate(&target, by_hash, reason).await {
                    // Terminated, or a benign no-op (already-terminated FoldRefused / absent None) — nothing
                    // more to do; the durable marker (if fresh) + registry removal are done inside terminate.
                    Some(Ok(_)) | Some(Err(KernelError::FoldRefused)) | None => {}
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
        }
    }
    Ok(())
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
        let (tx, rx) = mpsc::unbounded_channel();
        let (admin_tx, admin_rx) = mpsc::unbounded_channel();
        let (lifecycle_tx, lifecycle_rx) = mpsc::unbounded_channel();
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
        }
    }

    /// A cloneable [`LifecycleChannel`] sender to hand each session's [`LifecycleExecutor`](crate::LifecycleExecutor)
    /// (§lifecycle I5): the executor records `lifecycle/terminate` ops on it, and the loop drains + applies
    /// them after each `deliver`. Wire it into a session's executor set at spawn/install (with that session's
    /// id as the `owner`), the way the `Emit` executor gets [`inbox`](Self::inbox).
    pub fn lifecycle_channel(&self) -> LifecycleChannel {
        self.lifecycle_tx.clone()
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
        // it). A bounded buffer in practice (only accumulates while a target is suspended).
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
                                held_inbound.push(msg);
                                continue;
                            }
                            // A bounce is only ever needed for a cross-session Emit (reply_to set); an
                            // ordinary external inbound (reply_to None — the common case) never bounces. So
                            // capture the return-address + the echo payload ONLY when reply_to is set — an
                            // external inbound pays no PAYLOAD clone on the delivery hot path (Copilot #2408
                            // c2). (The `target` session-id clone just below is unconditional — it's the
                            // deliver-call handle, unavoidable since `msg.session` moves into `deliver`.)
                            // These must be taken BEFORE `msg.body` moves into deliver.
                            let bounce_ctx = msg
                                .reply_to
                                .clone()
                                .map(|sender| (sender, bounce_echo_payload(&msg.body)));
                            let target = msg.session.clone();
                            match host.deliver(&msg.session, msg.body, msg.cause).await {
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
                )
                .await;
            }

            // Apply any lifecycle ops a session's LifecycleExecutor recorded during this iteration's deliver
            // (§lifecycle I5 defer-to-loop): the executor couldn't mutate the registry from inside `perform`
            // (registry borrowed for the driven session), so it enqueued the op here — now `&mut host` is
            // free. Drained synchronously (the ops were produced on THIS task, so there's nothing to await).
            apply_lifecycle_ops(&mut host, &mut lifecycle_rx).await?;

            // A lifecycle op may have RESUMED a session (§lifecycle I4): replay any inbound held for a
            // now-un-suspended target. Deliver each in-place (still-suspended ones stay held). Partition:
            // keep still-held; deliver the rest. (A resumed target that terminated meanwhile → deliver returns
            // None/FoldRefused, both benign here — a held inbound to a gone session is dropped, not bounced,
            // since a held inbound has no live emitter awaiting it.)
            if !held_inbound.is_empty() {
                let mut still_held = Vec::new();
                for msg in std::mem::take(&mut held_inbound) {
                    if host.is_suspended(&msg.session) {
                        still_held.push(msg);
                    } else {
                        match host.deliver(&msg.session, msg.body, msg.cause).await {
                            Some(Ok(())) | None | Some(Err(KernelError::FoldRefused)) => {}
                            Some(Err(e)) => return Err(e),
                        }
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
) {
    let principal = req.principal.as_deref().unwrap_or("");
    let resp = host
        .apply_admin_authorized(req.command, principal, admin_authz, factory, Some(now_ms))
        .await;
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
        async fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
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
        host.spawn(SessionId::new("a"), mark_host());
        host.spawn(SessionId::new("b"), mark_host());
        let async_host = AsyncAgentHost::new(host);
        let inbox = async_host.inbox();

        let (_sd_tx, sd_rx) = tokio::sync::oneshot::channel();
        // Feed both sessions, then DROP the inbox → the loop drains the two messages and returns (all
        // senders gone). A fixed clock (no timers armed here) keeps it deterministic.
        inbox
            .send(Inbound {
                session: SessionId::new("a"),
                body: go(),
                cause: None,
                reply_to: None,
            })
            .unwrap();
        inbox
            .send(Inbound {
                session: SessionId::new("b"),
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
                host.get(&SessionId::new(id))
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
        async fn fold(&self, event: &Event, _kv: &mut Kv) -> FoldOutput {
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

    struct EmitterAgent {
        peer: String,
        message: Vec<u8>,
    }
    #[async_trait::async_trait(?Send)]
    impl Reducer for EmitterAgent {
        async fn fold(&self, event: &Event, _kv: &mut Kv) -> FoldOutput {
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
        async fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
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

        // A: EmitterAgent → emits to B on its trigger, dispatched by the REAL EmitExecutor over `tx`.
        // AUTHORIZED to Emit exactly to B (the kernel gates the emit before dispatch, SEC-F1 — an un-granted
        // target would be DENIED, and this proves the AUTHORIZED path).
        let mut a = HostedSession::genesis(
            Hash::of(b"emitter-v1"),
            Box::new(EmitterAgent {
                peer: "session-b".to_string(),
                message: b"hello-from-a".to_vec(),
            }),
            Box::new(Authorizer::new(vec![Capability {
                kind: EffectKind::Emit,
                predicate: ResourcePredicate::Exact("session-b".into()),
            }])),
            CompositeExecutor::new().with_effect(
                effect_ct::EMIT,
                Box::new(crate::EmitExecutor::new(
                    tx.clone(),
                    SessionId::new("session-a"),
                )),
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
            routed.session.as_str(),
            "session-b",
            "routed to the target peer session"
        );
        // The routed Inbound carries the EMITTER as reply_to (§lifecycle I5): this is the return-address the
        // loop bounces a delivery-failure to if B is gone/terminated. Assert it so a regression that drops or
        // mis-stamps reply_to (bounces would stop reaching the emitter) is caught (Copilot #2408 c3).
        assert_eq!(
            routed.reply_to.as_ref().map(|s| s.as_str()),
            Some("session-a"),
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
        async fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
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

    #[tokio::test]
    async fn an_emit_to_an_absent_target_bounces_a_delivery_failure_to_the_sender() {
        // §lifecycle I5 bounce: an Inbound produced by a cross-session Emit (reply_to = the sender) whose
        // TARGET is not registered (terminated→removed, or never spawned) does NOT silently drop — the loop
        // routes a `delivery-failure` Inbound back to the sender, which folds it. Drive the real loop.
        let mut host = AgentHost::new();
        // Only the SENDER is registered; the target "ghost" is absent (models a terminated-then-removed peer).
        host.spawn(SessionId::new("sender"), {
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
                session: SessionId::new("ghost"),
                body: EventBody::Inbound {
                    content_type: ContentType {
                        family: "message".into(),
                        version: 1,
                    },
                    payload: Payload::Inline(b"undeliverable".to_vec().into()),
                },
                cause: None,
                reply_to: Some(SessionId::new("sender")),
            })
            .unwrap();
        drop(inbox);

        let host = async_host
            .run(sd_rx, || 0)
            .await
            .expect("clean shutdown, no kernel error");

        // The sender folded a delivery-failure carrying the failed message's payload (correlation echo).
        assert_eq!(
            host.get(&SessionId::new("sender"))
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
        host.spawn(SessionId::new("live"), mark_host());
        let async_host = AsyncAgentHost::new(host);
        let inbox = async_host.inbox();
        let (_sd_tx, sd_rx) = tokio::sync::oneshot::channel();
        inbox
            .send(Inbound {
                session: SessionId::new("nobody"),
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
        assert!(host.contains(&SessionId::new("live")));
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
            SessionId::new("victim"),
            HostedSession::genesis(
                Hash::of(b"victim-v1"),
                Box::new(MarkAgent),
                Box::new(Authorizer::deny_all()),
                CompositeExecutor::new(),
            ),
        );
        // The CONTROLLER: a LifecycleExecutor over THIS loop's lifecycle channel (owner = "controller"),
        // authorized to lifecycle/terminate the victim. Fetched after wrapping (the channel lives on the loop).
        let controller = HostedSession::genesis(
            Hash::of(b"controller-v1"),
            Box::new(TerminatorAgent {
                victim: "victim".to_string(),
            }),
            // lifecycle/* is a register-by-string family (no dedicated EffectKind) → grant it via a
            // FAMILY grant (Capability::for_family), not a kind-based Capability. Authorized to
            // lifecycle/terminate exactly the victim.
            Box::new(
                Authorizer::new(vec![]).with_family_grants(vec![Capability::for_family(
                    effect_ct::LIFECYCLE_TERMINATE,
                    ResourcePredicate::Exact("victim".into()),
                )]),
            ),
            CompositeExecutor::new().with_effect(
                effect_ct::LIFECYCLE_TERMINATE,
                Box::new(crate::LifecycleExecutor::new(
                    async_host.lifecycle_channel(),
                    SessionId::new("controller"),
                )),
            ),
        );
        async_host
            .host_mut()
            .spawn(SessionId::new("controller"), controller);

        let inbox = async_host.inbox();
        let (_sd_tx, sd_rx) = tokio::sync::oneshot::channel();
        // Trigger the controller → it emits lifecycle/terminate(victim). Drop inbox → loop drains + returns.
        inbox
            .send(Inbound {
                session: SessionId::new("controller"),
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
            !host.contains(&SessionId::new("victim")),
            "the victim was terminated + removed from the registry via lifecycle/terminate through the loop"
        );
        // The controller itself is untouched (it terminated a PEER, not itself).
        assert!(host.contains(&SessionId::new("controller")));
    }

    /// A timer agent: arms a timer at `deadline_ms` on "go"; records "woke" when it fires (PR#1303 fix).
    struct TimerAgent {
        deadline_ms: u64,
    }
    #[async_trait::async_trait(?Send)]
    impl Reducer for TimerAgent {
        async fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
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
            SessionId::new("t"),
            HostedSession::genesis(
                Hash::of(b"timer-v1"),
                Box::new(TimerAgent { deadline_ms: 1000 }),
                Box::new(authz),
                CompositeExecutor::new(),
            ),
        );
        // Arm the timer directly (pre-run) so it's already armed when the loop starts.
        host.deliver(&SessionId::new("t"), go(), None).await;
        assert_eq!(
            host.get(&SessionId::new("t"))
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
            host.get(&SessionId::new("t"))
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
        host.spawn(SessionId::new("wall"), mark_host());
        let async_host = AsyncAgentHost::new(host);
        let inbox = async_host.inbox();
        let (_sd_tx, sd_rx) = tokio::sync::oneshot::channel();
        inbox
            .send(Inbound {
                session: SessionId::new("wall"),
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
            host.get(&SessionId::new("wall"))
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
    }

    fn install(id: &str) -> AdminCommand {
        AdminCommand::InstallSession(InstallSpec {
            id: SessionId::new(id),
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
                    id: SessionId::new("a")
                }
            );
            assert_eq!(
                admin_call(&admin, install("b")).await,
                AdminResponse::Installed {
                    id: SessionId::new("b")
                }
            );
            assert_eq!(
                admin_call(&admin, AdminCommand::ListSessions).await,
                AdminResponse::Sessions {
                    ids: vec![SessionId::new("a"), SessionId::new("b")]
                }
            );
            // A stop, then list reflects the removal.
            assert_eq!(
                admin_call(
                    &admin,
                    AdminCommand::StopSession {
                        id: SessionId::new("a")
                    }
                )
                .await,
                AdminResponse::Stopped {
                    id: SessionId::new("a")
                }
            );
            assert_eq!(
                admin_call(&admin, AdminCommand::ListSessions).await,
                AdminResponse::Sessions {
                    ids: vec![SessionId::new("b")]
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
}
