//! The kernel core — `fold → authorize → durably-dispatch → execute → fold result` (§2).
//!
//! This is the v0.1 spine, single-session and in-memory, with the correctness-critical invariants from
//! the adversarial review designed in:
//!
//! - **S1 (durable dispatch):** before an effect is handed to an executor, a `Dispatched` event is
//!   appended to the log, and recovery re-drives un-resulted dispatches by idempotency key so a crash
//!   between dispatch and result never double-fires or drops. A `Session` with a [`crate::log_store::LogStore`]
//!   attached ([`Session::attach_log`]) WRITES THROUGH on every append (persist before the hash is
//!   returned + used to route), so the durable-before-route ordering holds in-kernel — and the drive
//!   loop refuses to route an effect whose `Dispatched` frame failed to persist (it records a
//!   failed-undurable result instead of calling the executor). This is durability tier B (latched, via
//!   `persist_error` / [`Session::take_persist_error`]); tier A (strict fallible-abort of the whole
//!   drive) is the tracked hardening for when irreversible external routing goes live in anger. Recovery
//!   (`replay`/`recover`) rebuilds KV + the open-obligation set from the log.
//! - **S4 (effect-id correlation + timeout-cancels):** each effect gets a monotonic `EffectId`; results
//!   fold back correlated by id. A timeout cancels the dispatch — once an outcome (Ok/Err/TimedOut) is
//!   recorded for an id, no second outcome for that id is ever accepted.
//! - **SEC-F1 (resource-scoped authz):** every effect is checked against a capability whose predicate
//!   gates the resolved target, not just the effect kind. A denied effect is logged, never executed.
//!
//! The KV is rebuilt by folding the log (it IS derived state — §4); a snapshot is `(seq, kv.root_hash,
//! reducer_hash)`.

use crate::authz::Authorize;
use crate::effect::{EffectId, EffectKind, EffectRequest};
use crate::event::{EffectOutcome, Event, EventBody};
use crate::executor::Executor;
use crate::hash::Hash;
use crate::kv::Kv;
use crate::reducer::{Effect, Reducer};
use std::collections::{BTreeMap, BTreeSet};
use std::io;

/// Errors the kernel surfaces to its driver. Kept small; grows with features.
#[derive(Debug, PartialEq, Eq)]
pub enum KernelError {
    /// An event was appended out of sequence (log corruption / programming error).
    NonContiguousSeq { expected: u64, got: u64 },
    /// The first event of a session must be `Genesis`.
    MissingGenesis,
}

/// A single-session kernel instance: the authoritative log plus the derived KV and the id counter.
/// The reducer/executor/authorizer are supplied per operation so the same log can be replayed under a
/// pinned reducer (the §16c-S3 "replay under the version that wrote it" discipline).
pub struct Session {
    log: Vec<Event>,
    kv: Kv,
    /// Next effect id to assign. Monotonic within the session (§16c-S4). Derived from the log on
    /// replay so it never collides after recovery.
    next_effect_id: u64,
    /// Effect ids that have a *terminal* outcome recorded (Ok/Err/TimedOut). Used to enforce
    /// timeout-cancels: a late result for a settled id is dropped (§16c-S4).
    settled: BTreeSet<u64>,
    /// Effect ids that have been dispatched but not yet settled — the crash-recovery obligation set
    /// (§16c-S1). Populated during replay; drained as results/timeouts fold in.
    open: BTreeSet<u64>,
    /// Armed-but-unfired timers: effect id → ABSOLUTE deadline in wall-clock ms (§9c/§16c-S5). The
    /// absolute anchor (not a duration) is what lets a recovered/migrated session compute remaining
    /// time. Rebuilt from `TimerArmed` events on replay; drained when the timer fires. The kernel — not
    /// an executor — injects `TimerFired` once `now_ms` reaches the deadline (see `fire_due_timers`).
    armed_timers: BTreeMap<u64, u64>,
    /// The last `Now`-effect timestamp this session HANDED BACK, in binary nanoseconds since epoch —
    /// the monotonicity high-water mark (operator ruling). The `Now` clock effect must be strictly
    /// increasing: a raw wall-clock reading `<= last_now` is clamped up to `last_now + 1` before it's
    /// recorded, so successive `now()`s never repeat or go backwards (wall-clock resolution / NTP steps
    /// can't break log ordering). The kernel stays CLOCK-FREE (§9c) — the executor reads the raw clock;
    /// this only CLAMPS the value handed to it. Rebuilt from the log's recorded (already-clamped) `Now`
    /// results on replay so `last_now` is replay-deterministic (like `next_effect_id`/`armed_timers`).
    last_now: u64,
    /// The durable log this session writes THROUGH as it appends (§16c-S1), if attached via
    /// [`Session::attach_log`]. When present, every appended event is persisted (append + flush) before
    /// `append` returns — so the S1 "Dispatched durable before its effect routes" ordering is enforced
    /// IN-KERNEL, not left to a driver mirroring events by hand — UNLESS a prior persist failure is
    /// latched (durability tier B): `append` records the first `store` error into `persist_error`
    /// (first-error-wins), then returns the hash and SKIPS further writes, so after a failure the S1
    /// guarantee holds only while [`Session::take_persist_error`] is `None` (which the driver MUST
    /// check). Past a failure the on-disk log stops at the last good frame and recovery heals the tail
    /// (`truncate_to`). `None` = an in-memory-only session (tests, or a caller that persists
    /// separately). Persistence lives here, next to the in-memory log it shadows, so the two never
    /// diverge while durable.
    store: Option<Box<dyn crate::log_store::LogSink>>,
    /// The first persistence error hit while writing through `store`, latched here (§16c-S1, tier B —
    /// see [`Session::attach_log`]). The in-memory log + fold always succeed, so `append`/`drive` stay
    /// infallible; a disk write failure is recorded (not swallowed, not panicked) and surfaced to the
    /// driver via [`Session::take_persist_error`], which it MUST check after a `deliver`/`fire_due_timers`
    /// before acting on the run's external effects as durably-logged. Latched (first error wins) so one
    /// failure doesn't spam; once set, further writes are skipped (the on-disk log stopped at the last
    /// good frame, which recovery heals via `truncate_to`).
    persist_error: Option<io::Error>,
}

impl Session {
    /// Start a fresh session with a genesis event naming the reducer. The genesis is the first log
    /// entry; nothing is folded yet (genesis carries no effects).
    pub fn genesis(reducer: Hash) -> Self {
        let mut s = Session {
            log: Vec::new(),
            kv: Kv::new(),
            next_effect_id: 0,
            settled: BTreeSet::new(),
            open: BTreeSet::new(),
            armed_timers: BTreeMap::new(),
            last_now: 0,
            store: None,
            persist_error: None,
        };
        s.log.push(Event {
            seq: 0,
            cause: None,
            body: EventBody::Genesis { reducer },
        });
        s
    }

    /// Attach a durable [`crate::log_store::LogStore`] so this session WRITES THROUGH it on every append
    /// (§16c-S1, durability tier B — see the `store`/`persist_error` fields). The store should already
    /// hold this session's log up to the current tip (e.g. it was just used to [`Session::recover`], or
    /// it's an empty store for a fresh `genesis`); attaching does NOT re-persist the existing in-memory
    /// log. From here, each appended event is persisted before `append` returns, so the S1
    /// "Dispatched-durable-before-route" ordering holds in-kernel. A persist failure is latched (see
    /// [`Session::take_persist_error`]), not propagated — the in-memory session stays consistent.
    ///
    /// **Durability tier (concierge ruling): B (latched) is the v0 baseline; A (strict abort) is the
    /// committed hardening for when the kernel routes irreversible external effects in anger.** In tier
    /// B, the kernel already refuses to ROUTE an effect whose `Dispatched` frame failed to persist (the
    /// drive loop checks the latch before `executor.perform` — the actual S1 danger, an un-doable route
    /// on an un-durable dispatch). The driver's remaining obligation: after a run, call
    /// [`Session::take_persist_error`]; if `Some`, the run's log is not fully durable (recovery heals the
    /// torn tail via `truncate_to`) — surface/alert, don't silently continue.
    pub fn attach_log(&mut self, store: crate::log_store::LogStore) {
        self.store = Some(Box::new(store));
    }

    /// Attach an arbitrary [`crate::log_store::LogSink`] as the write-through target — the generic form
    /// of [`Session::attach_log`]. Lets a caller supply a non-`LogStore` sink (a network/replicated log,
    /// or — in tests — a sink that fails its append so the S1 route-guard can be exercised).
    pub fn attach_sink(&mut self, sink: Box<dyn crate::log_store::LogSink>) {
        self.store = Some(sink);
    }

    /// Take the latched persistence error, if any (§16c-S1 tier B). Call after a
    /// `deliver`/`fire_due_timers`/`time_out_effect`; `Some` means a write-through failed mid-run, so the
    /// on-disk log stopped at the last good frame (recovery heals the torn tail via `truncate_to`) —
    /// alert/surface, don't silently proceed. Note the kernel ALREADY prevents the core S1 hazard: an
    /// effect whose `Dispatched` frame didn't persist is NOT routed (its outcome is a failed-undurable
    /// `EffectResult::Err`, not an executor call). Clears the latch. `None` = every appended event
    /// persisted.
    pub fn take_persist_error(&mut self) -> Option<io::Error> {
        self.persist_error.take()
    }

    pub fn kv(&self) -> &Kv {
        &self.kv
    }

    pub fn log(&self) -> &[Event] {
        &self.log
    }

    /// The current snapshot descriptor (§4): the free per-event checkpoint.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            seq: self.log.last().map(|e| e.seq).unwrap_or(0),
            kv_root: self.kv.root_hash(),
            reducer: self.reducer_hash(),
        }
    }

    /// Fork this session into a fresh, EPHEMERAL query session over its CURRENT materialized state — the
    /// kernel half of the operator's fork-for-query session-debug mechanism (the semantic complement to the
    /// structural [`Session::status_snapshot`]). The fork is a brand-new session with its OWN genesis and
    /// id-space, seeded with a CLONE of this session's current KV and the SAME reducer-hash, so it's ready
    /// to fold a "summarize yourself" inbound message WITHOUT replaying history (fork-from-snapshot, not
    /// full-replay — the materialized KV *is* the snapshot at the current seq). The caller then
    /// [`Session::deliver`]s a report/summarize message, runs to quiescence, reads the reducer's answer
    /// (its `public/` view or a model result), and DISCARDS the fork — it is never persisted.
    ///
    /// NON-INTERFERENCE by construction: this reads `self` immutably (a KV clone + the reducer-hash) and
    /// returns a SEPARATE session; the query events land in the fork's log only. The original's log, KV,
    /// open obligations, and armed timers are untouched — it never sees the query, never folds it, never
    /// forks its train of thought. That's the whole point of forking rather than injecting into the live
    /// session.
    ///
    /// The fork starts with NO in-flight obligations (empty open/settled/armed-timer sets, `next_effect_id`
    /// reset): it doesn't inherit the parent's pending effects — it's a clean reactive session over the
    /// materialized state whose only job is to answer the query. It carries the parent's monotonic clock
    /// floor (`last_now`) so a `Now` read in the fork can't observe time earlier than the parent already
    /// did. It attaches NO log store (ephemeral — a query artifact, never durable) and clears any latched
    /// persist error.
    ///
    /// AUTHZ (host obligation, not enforced here): the fork should be driven with a MODEL-ONLY capability
    /// so a query fold can't take world-actions (no Http/Shell/Emit leaking from a debug query) — the
    /// kernel supplies the mechanism (a clean isolated session); the caller supplies the scoped `Authorize`.
    pub fn fork_for_query(&self) -> Session {
        // Build the clean base via `genesis` so the fork can't DRIFT from the canonical construction if
        // `Session` gains a field (PR#1297 review): a fresh session over the SAME reducer already gives
        // empty open/settled/armed-timer sets, `next_effect_id` reset, no log store, and no latched persist
        // error. Then override only the fork DELTAS: the materialized KV (cloned) and the monotonic clock
        // floor (so a `Now` read in the fork can't observe time earlier than the parent already did).
        let mut fork = Session::genesis(self.reducer_hash());
        fork.kv = self.kv.clone();
        fork.last_now = self.last_now;
        fork
    }

    /// The reducer this session was created with (from genesis). FAILS LOUDLY on a log whose first
    /// event isn't Genesis (PR#990 finding #2): both `genesis()` and `replay()` guarantee a Genesis
    /// first event, so a missing one is corruption, not a normal state — masking it with a bogus
    /// `Hash::of(b"")` would produce a misleading snapshot. This is an internal invariant, so a panic is
    /// the right failure (a corrupt in-memory session is a bug, not a recoverable input — untrusted log
    /// bytes are already rejected by `replay`/`decode` before a Session exists).
    fn reducer_hash(&self) -> Hash {
        match self.log.first().map(|e| &e.body) {
            Some(EventBody::Genesis { reducer }) => *reducer,
            _ => panic!(
                "cdz-kernel invariant violated: session log's first event is not Genesis \
                 (a Session is only constructed via genesis()/replay(), both of which guarantee it)"
            ),
        }
    }

    /// The count of dispatched-but-unsettled effects — the anti-stuck / recovery signal (§4b tier-2).
    pub fn open_effects(&self) -> usize {
        self.open.len()
    }

    /// Assemble a structural [`StatusSnapshot`] — the CHEAP, non-interfering session-debug read (operator
    /// session-query design). Reads ONLY already-materialized state (the log, the open-obligation set, the
    /// armed-timer table, the published KV view): it appends NO event, runs NO fold, so the session can't
    /// be derailed and doesn't know it was asked. This is the "is X alive/stalled/idle?" answer for free;
    /// the semantic "what is X DOING?" answer is a fork-for-query (a fork's model summarizes itself).
    ///
    /// `now_ms` is passed by the CALLER (the kernel stays clock-free — §9c — it never reads the clock
    /// itself; see `fire_due_timers`): it's used only to derive [`SessionState::Stalled`] from how long an
    /// in-flight effect has been outstanding. `stall_after_ms` is the staleness threshold (e.g. 5 min); an
    /// in-flight effect whose dispatch is older than it flips the state to `Stalled`. Passing `None` for
    /// `now_ms` skips stall detection (state is Active/Quiescent/Closed only) — for a caller with no clock.
    pub fn status_snapshot(&self, now_ms: Option<u64>, stall_after_ms: u64) -> StatusSnapshot {
        // In-flight effects: each open id's DURABLE Dispatched frame carries its kind + target (what it's
        // waiting on) + the dispatch deadline anchor. Scan the log once, keep those whose id is still open.
        let mut in_flight = Vec::new();
        let mut oldest_dispatch_ms: Option<u64> = None;
        for e in &self.log {
            if let EventBody::Dispatched {
                id,
                kind,
                target,
                deadline_ms,
                ..
            } = &e.body
            {
                if self.open.contains(&id.0) {
                    in_flight.push(InFlight {
                        kind: effect_kind_name(kind),
                        target: target.clone(),
                    });
                    // The dispatch's deadline anchor (if any) doubles as its dispatch-time reference for
                    // stall detection; track the oldest so a long-outstanding effect trips Stalled.
                    if let Some(d) = deadline_ms {
                        oldest_dispatch_ms =
                            Some(oldest_dispatch_ms.map_or(*d, |o: u64| o.min(*d)));
                    }
                }
            }
        }

        let closed = self
            .log
            .iter()
            .any(|e| matches!(e.body, EventBody::Closed { .. }));
        let has_work = !self.open.is_empty() || !self.armed_timers.is_empty();
        // Stall: an in-flight effect outstanding longer than the threshold (only derivable with a clock).
        let stalled = match (now_ms, oldest_dispatch_ms) {
            (Some(now), Some(oldest)) if !self.open.is_empty() => {
                now.saturating_sub(oldest) > stall_after_ms
            }
            _ => false,
        };
        let state = if closed {
            SessionState::Closed
        } else if stalled {
            SessionState::Stalled
        } else if has_work {
            SessionState::Active
        } else {
            SessionState::Quiescent
        };

        // Tier-1 published view: the session's KV entries under the `public/` prefix — the semantic status
        // it CHOSE to expose (the full KV is higher-privilege, not surfaced here).
        let published = self
            .kv
            .prefix_scan(b"public/")
            .into_iter()
            .map(|(k, v)| (k.to_vec(), v.to_vec()))
            .collect();

        StatusSnapshot {
            state,
            event_count: self.log.len() as u64,
            last_event_kind: self
                .log
                .last()
                .map(|e| event_body_name(&e.body))
                .unwrap_or("(empty)"),
            in_flight,
            armed_timers: self.armed_timers.len() as u32,
            published,
        }
    }

    /// Deliver an inbound event and drive the fold→authorize→dispatch→fold-result loop to quiescence,
    /// awaiting the reducer's fold via [`Reducer`] — the async form of [`Session::deliver`]. Appends
    /// `body` (cause-linked to `cause`), then folds it through `reducer`; each effect the fold emits is
    /// authorized then handled by kind: an executor-dispatched effect (Http/Model/Shell/Now/Emit) is
    /// durably dispatched, performed by the executor, and its result folded back; a `Timer` effect is
    /// ARMED (a durable `TimerArmed`, no executor call) and fired later by [`Session::fire_due_timers_async`].
    /// The loop runs until no new effects remain. Because the fold is `.await`ed, a long-running wasm fold
    /// cooperatively YIELDS at fuel intervals (see [`crate::wasm_host::AsyncComponentReducer`]) rather than
    /// blocking the caller's single-threaded loop.
    ///
    /// The executor is invoked synchronously (`&mut dyn Executor`): only the reducer fold awaits. The
    /// guest-facing WIT ABI is blocking — the async lives entirely in the host-side Rust driving the
    /// component, never in the guest's interface.
    pub async fn deliver_async(
        &mut self,
        body: EventBody,
        cause: Option<Hash>,
        reducer: &dyn Reducer,
        authz: &(impl Authorize + ?Sized),
        executor: &mut (impl Executor + ?Sized),
    ) -> Result<(), KernelError> {
        // The common delivery path DROPS the surfaced control/* effects (a live session turn doesn't
        // consume them by default). A driver that needs them — `fork_for_query`'s summary watch — calls
        // [`Session::deliver_async_control`] instead. Keeping this `-> Result<(), _>` is the never-red
        // bridge: the downstream `cdz-agent-host` HostedSession::deliver returns this verbatim, so widening
        // it would break its build; the control-returning variant is ADDITIVE alongside it.
        self.deliver_async_control(body, cause, reducer, authz, executor)
            .await
            .map(|_control| ())
    }

    /// Like [`Session::deliver_async`], but RETURNS the `control/*` effects the reducer emitted this turn
    /// (control-plane partition, register-by-string): `control/*` families are authz-exempt + not routed —
    /// the kernel collects them and hands them back here for the DRIVER to consume (e.g. `fork_for_query`
    /// scrapes the `control/summary` effect's `request.payload`). The common [`deliver_async`] path drops
    /// them; use this when you need them. See [`crate::effect::ControlEffect`].
    pub async fn deliver_async_control(
        &mut self,
        body: EventBody,
        cause: Option<Hash>,
        reducer: &dyn Reducer,
        authz: &(impl Authorize + ?Sized),
        executor: &mut (impl Executor + ?Sized),
    ) -> Result<Vec<crate::effect::ControlEffect>, KernelError> {
        self.append(body, cause);
        let control = self.drive_async(reducer, authz, executor).await;
        Ok(control)
    }

    /// The ASYNC twin of [`Session::fire_due_timers`] — fire every armed timer past `now_ms`, driving the
    /// [`Reducer`] for each. Same determinism (§9c): the FIRED time is the timer's own deadline, not
    /// `now_ms`, so replay reconstructs identically. Additive alongside the sync `fire_due_timers`.
    pub async fn fire_due_timers_async(
        &mut self,
        now_ms: u64,
        reducer: &dyn Reducer,
        authz: &(impl Authorize + ?Sized),
        executor: &mut (impl Executor + ?Sized),
    ) -> usize {
        let mut due: Vec<(u64, u64)> = self
            .armed_timers
            .iter()
            .filter(|(_, &deadline)| deadline <= now_ms)
            .map(|(&id, &deadline)| (deadline, id))
            .collect();
        due.sort_unstable();
        for (deadline, id) in &due {
            let token = self.timer_armed_token_of(EffectId(*id)).unwrap_or(None);
            self.append(
                EventBody::TimerFired {
                    id: EffectId(*id),
                    fired_ms: *deadline,
                    token,
                },
                None,
            );
            self.drive_async(reducer, authz, executor).await;
        }
        due.len()
    }

    /// Absolute deadlines of the currently armed-but-unfired timers (§16c-S5), for the driver's
    /// scheduler (it sleeps until the earliest) and for tests.
    pub fn next_timer_deadline(&self) -> Option<u64> {
        self.armed_timers.values().copied().min()
    }

    /// Append one event to the authoritative log at the next sequence, folding it into KV. Does NOT
    /// perform effects — that's `drive`. Returns the appended event's hash (for `cause` linking).
    ///
    /// Append is INFALLIBLE at the type level: pushing onto the in-memory log cannot fail, so it returns
    /// the new event's `Hash` directly (no `Result` to `let _ =`-swallow — the review flagged that as a
    /// latent trap). DURABLE write-through (§16c-S1, tier B): if a [`crate::log_store::LogStore`] is
    /// attached ([`Session::attach_log`]), the event is persisted (append + flush) HERE, before the hash
    /// is returned and thus before `drive` uses it to route the effect — the S1 "Dispatched durable
    /// before route" ordering. A persist failure is LATCHED into `persist_error` (first error wins) and
    /// further writes are skipped, rather than propagated — the in-memory session stays consistent and
    /// the driver observes the failure via [`Session::take_persist_error`]. (Strict abort-on-persist-fail
    /// — tier A — is a raised design decision; this is the tier-B baseline.)
    fn append(&mut self, body: EventBody, cause: Option<Hash>) -> Hash {
        // Maintain the open/settled sets and the armed-timer table as obligations are created and
        // discharged (§16c-S1/S4/S5).
        match &body {
            EventBody::Dispatched { id, .. } => {
                self.open.insert(id.0);
            }
            EventBody::TimerArmed {
                id, deadline_ms, ..
            } => {
                self.open.insert(id.0);
                self.armed_timers.insert(id.0, *deadline_ms);
            }
            EventBody::EffectResult { id, .. } => {
                self.open.remove(&id.0);
                self.settled.insert(id.0);
            }
            EventBody::TimerFired { id, .. } => {
                self.open.remove(&id.0);
                self.armed_timers.remove(&id.0);
                self.settled.insert(id.0);
            }
            _ => {}
        }
        let seq = self.log.len() as u64;
        let event = Event { seq, cause, body };
        let hash = event.hash();
        // Durable write-through BEFORE returning the hash (so the caller routes only after the event is
        // on disk — S1). Skip once an error is latched (the on-disk log stopped at the last good frame;
        // recovery heals the tail). First error wins; recorded, never swallowed or panicked.
        if self.persist_error.is_none() {
            if let Some(store) = self.store.as_mut() {
                if let Err(e) = store.append(&event) {
                    self.persist_error = Some(e);
                }
            }
        }
        self.log.push(event);
        hash
    }

    /// The ASYNC twin of [`Session::drive`] — same fold→authorize→dispatch→fold-result loop, but the
    /// reducer folds are `.await`ed (via [`Reducer`]) so a long wasm fold cooperatively yields. The
    /// authorize/executor/append mechanics are IDENTICAL to the sync path — only the fold calls await.
    async fn drive_async(
        &mut self,
        reducer: &dyn Reducer,
        authz: &(impl Authorize + ?Sized),
        executor: &mut (impl Executor + ?Sized),
    ) -> Vec<crate::effect::ControlEffect> {
        let trigger = self.tip_hash();
        let initial = self.fold_tip_async(reducer, trigger).await;
        self.drive_worklist_async(initial, reducer, authz, executor)
            .await
    }

    /// The ASYNC twin of [`Session::drive_worklist`] — structurally identical (authorize → durable
    /// dispatch → execute → fold-result), but the reducer folds (`fold_tip_async`/`record_result_async`)
    /// are `.await`ed. The executor call stays SYNC (only the reducer fold yields this slice); the
    /// authorize/timer/append/S1-latch logic is byte-identical to the sync path. Kept as a parallel copy
    /// because the fold-call interleaving (which mutates `to_process` mid-loop) can't be factored behind a
    /// sync/async-agnostic helper without threading a closure per fold point — the duplication is the
    /// never-red migration cost, removed with the sync path at step 6.
    async fn drive_worklist_async(
        &mut self,
        mut to_process: Vec<(Effect, Hash)>,
        reducer: &dyn Reducer,
        authz: &(impl Authorize + ?Sized),
        executor: &mut (impl Executor + ?Sized),
    ) -> Vec<crate::effect::ControlEffect> {
        let mut control_out = Vec::new();
        while let Some((effect, cause)) = to_process.pop() {
            let Effect {
                request: req,
                token,
            } = effect;
            let id = EffectId(self.next_effect_id);
            self.next_effect_id += 1;

            // CONTROL-PLANE PARTITION (register-by-string): a `control/*` family is authz-EXEMPT and NEVER
            // routed to an executor — it's host-answered. `control/capabilities` is KERNEL-answered INLINE
            // (host-capability-discovery I4): the kernel projects the capability manifest and folds it back
            // as an EffectResult, so the guest gets its answer without a host round-trip. Every OTHER
            // control/* family surfaces to the driver (via the returned Vec) — host-answered.
            if crate::effect::effect_ct::is_control_family(&req.content_type.family) {
                if req.content_type.family.as_ref() == crate::effect::effect_ct::CAPABILITIES {
                    // Durable Dispatched record BEFORE the answer (S1) — also what gives
                    // record_result_async the continuation token to resume the guest. authz-exempt: no
                    // authorize() call; the projection PROBES authz per family but the query itself is free.
                    let idempotency_key = idempotency_key_for(id, &req);
                    let dispatch_hash = self.append(
                        EventBody::Dispatched {
                            id,
                            kind: req.kind.clone(),
                            target: req.target.to_string(),
                            idempotency_key,
                            deadline_ms: None,
                            token,
                        },
                        Some(cause),
                    );
                    // S1 latch: an un-durable dispatch folds an Err, never a phantom answer (same tier-B
                    // rule the routed path applies before executing).
                    let outcome = if self.persist_error.is_some() {
                        EffectOutcome::Err(
                            "dispatch not durably logged (persist failure) — capabilities NOT answered (S1)"
                                .to_string(),
                        )
                    } else {
                        // Project over the canonical family set: the MECHANISM dim is the executor's
                        // handles_family (now on the Executor trait so it's reachable through
                        // &dyn Executor), the POLICY dim probes authz per family. The manifest BYTES are
                        // logged in the EffectResult, so replay reads the logged answer — deterministic, it
                        // never re-probes live executor/authz state.
                        let manifest = crate::effect::project_manifest(
                            crate::effect::effect_ct::ALL,
                            |f| executor.handles_family(f),
                            authz,
                            crate::effect::effect_ct::probe_target,
                        )
                        .await;
                        let bytes = crate::event_ast::encode_capability_manifest(&manifest);
                        EffectOutcome::Ok(Some(crate::effect::Payload::Inline(bytes.into())))
                    };
                    let more = self
                        .record_result_async(id, outcome, reducer, dispatch_hash)
                        .await;
                    for pair in more {
                        to_process.push(pair);
                    }
                    continue;
                }
                control_out.push(crate::effect::ControlEffect {
                    request: req,
                    token,
                });
                continue;
            }

            // SEC-F1: authorize against the resolved target (same as sync path), awaiting the async gate.
            if let Err(reason) = authz.authorize_async(&req).await {
                let denial_hash =
                    self.append(EventBody::AuthzDenied { id, reason, token }, Some(cause));
                for pair in self.fold_tip_async(reducer, denial_hash).await {
                    to_process.push(pair);
                }
                continue;
            }

            // Timers arm a kernel-fired deadline (§9c), not an executor call — same as sync path.
            if req.kind == EffectKind::Timer {
                match req.target.parse::<u64>() {
                    Ok(deadline_ms) => {
                        self.append(
                            EventBody::TimerArmed {
                                id,
                                deadline_ms,
                                token,
                            },
                            Some(cause),
                        );
                    }
                    Err(_) => {
                        let denial_hash = self.append(
                            EventBody::AuthzDenied {
                                id,
                                reason: format!("timer deadline not a u64 ms: {:?}", req.target),
                                token,
                            },
                            Some(cause),
                        );
                        for pair in self.fold_tip_async(reducer, denial_hash).await {
                            to_process.push(pair);
                        }
                    }
                }
                continue;
            }

            let idempotency_key = idempotency_key_for(id, &req);

            // S1: durable dispatch record BEFORE routing (same as sync path).
            let dispatch_hash = self.append(
                EventBody::Dispatched {
                    id,
                    kind: req.kind.clone(),
                    target: req.target.to_string(),
                    idempotency_key,
                    deadline_ms: None,
                    token,
                },
                Some(cause),
            );

            // S1 latch-check BEFORE routing (tier B): an un-durable dispatch is NOT routed (same as sync).
            if self.persist_error.is_some() {
                let outcome = EffectOutcome::Err(
                    "dispatch not durably logged (persist failure) — effect NOT routed (S1)"
                        .to_string(),
                );
                let more = self
                    .record_result_async(id, outcome, reducer, dispatch_hash)
                    .await;
                for pair in more {
                    to_process.push(pair);
                }
                continue;
            }

            // Route + execute — awaiting the async executor (a real I/O executor yields here without
            // blocking the single-threaded loop). The Dispatched record is already durable, so ordering
            // is preserved across the await.
            let outcome = executor.perform_async(&req, idempotency_key).await;

            // MONOTONIC `now` clamp (operator ruling) — same as sync path; only `Now` results are clamped.
            let outcome = if req.kind == EffectKind::Now {
                clamp_now_outcome(outcome, &mut self.last_now)
            } else {
                outcome
            };

            let more = self
                .record_result_async(id, outcome, reducer, dispatch_hash)
                .await;
            for pair in more {
                to_process.push(pair);
            }
        }
        control_out
    }

    /// The ASYNC twin of [`Session::fold_tip`] — folds the tip through a [`Reducer`] (`.await`),
    /// same FoldFailed capture + effect-reversal. This is the ONE place the reducer actually awaits.
    async fn fold_tip_async(&mut self, reducer: &dyn Reducer, cause: Hash) -> Vec<(Effect, Hash)> {
        let tip = self.log.last().expect("log always has genesis").clone();
        let out = reducer.fold_async(&tip, &mut self.kv).await;
        // Error-resilience (§17): a failed fold is captured as a FoldFailed log event, not folded further
        // — identical to the sync `fold_tip`.
        if let Some(reason) = out.failure {
            self.append(
                EventBody::FoldFailed {
                    reason,
                    caused_event: cause,
                },
                Some(cause),
            );
            return Vec::new();
        }
        let mut v: Vec<(Effect, Hash)> = out.effects.into_iter().map(|e| (e, cause)).collect();
        v.reverse();
        v
    }

    /// The ASYNC twin of [`Session::record_result`] — same timeout-cancels (drop a late result for a
    /// settled id) + token-copy-from-Dispatched-frame invariant, but folds the result through an
    /// [`Reducer`] (`.await`).
    async fn record_result_async(
        &mut self,
        id: EffectId,
        outcome: EffectOutcome,
        reducer: &dyn Reducer,
        dispatch_hash: Hash,
    ) -> Vec<(Effect, Hash)> {
        if self.settled.contains(&id.0) {
            return Vec::new();
        }
        let token = self.dispatch_token_of(id).unwrap_or_else(|| {
            panic!(
                "cdz-kernel invariant violated: EffectResult for {id:?} has no Dispatched frame to \
                 derive its continuation token from (§19b/§19e (B))"
            )
        });
        let result_hash = self.append(
            EventBody::EffectResult {
                id,
                result: outcome,
                token,
            },
            Some(dispatch_hash),
        );
        self.fold_tip_async(reducer, result_hash).await
    }

    /// Time out an open (dispatched-but-unsettled) effect — the missing half of the S4 recovery
    /// contract. [`Session::recover`] hands the driver a set of `open_effects` it must "re-drive OR time
    /// out"; this is the time-out half. A dispatch that will never get a result — a genuinely-outstanding
    /// call after a crash, or a hung async effect past its deadline — is settled here as
    /// [`EffectOutcome::TimedOut`], folded observably (so the reducer resumes its continuation with a
    /// timeout, and live-kv == replayed-kv — the same §9d anti-stuck outcome on both paths), and any
    /// effects that fold emits join the drive to quiescence.
    ///
    /// Timeout-cancels (§16c-S4): idempotent + monotonic. Timing out an id that's already settled (a real
    /// result beat the timeout, or it was already timed out) is a no-op returning `false` — never a
    /// second outcome for one id, so a continuation resumes at most once. Timing out an id that was never
    /// dispatched is likewise `false` (nothing open to cancel). Returns `true` iff this call settled it.
    ///
    /// The result event is `cause`-linked to the original `Dispatched` (found in the log), preserving the
    /// causal DAG (§5): trigger → dispatch → (timeout) result, exactly as a real result would link.
    pub async fn time_out_effect(
        &mut self,
        id: EffectId,
        reducer: &dyn Reducer,
        authz: &(impl Authorize + ?Sized),
        executor: &mut (impl Executor + ?Sized),
    ) -> bool {
        // Idempotent: only an OPEN id can be timed out. Settled (or never-dispatched) → no-op, so a late
        // real result and a timeout can't both settle one id (§16c-S4 at-most-once).
        if !self.open.contains(&id.0) {
            return false;
        }
        // `open` holds BOTH dispatched-effect ids AND armed-timer ids — but only a DISPATCHED effect can
        // be timed out (a timer isn't a hung external call; it fires via `fire_due_timers_async`). A timer
        // id has no `Dispatched` event, so `dispatch_hash_of` is None → return false (Copilot PR#1016: the
        // old code panicked here, contradicting the "never dispatched → false" contract). Timing out a
        // timer is a no-op, not a crash.
        let Some(dispatch_hash) = self.dispatch_hash_of(id) else {
            return false;
        };
        // Link the timeout result to the dispatch that opened it (causal DAG §5), like a real result.
        let more = self
            .record_result_async(id, EffectOutcome::TimedOut, reducer, dispatch_hash)
            .await;
        // The reducer's timeout continuation may emit further effects — drive them to quiescence.
        self.drive_worklist_async(more, reducer, authz, executor)
            .await;
        true
    }

    /// The hash of the `Dispatched` event that opened effect `id`, or `None` if `id` has no `Dispatched`
    /// event — which happens for an armed TIMER id (also in `open`, but opened by `TimerArmed`, not
    /// `Dispatched`). Callers that only mean dispatched effects (e.g. `time_out_effect`) treat `None` as
    /// "not a dispatched effect" rather than an error (PR#1016 — `open` is a mixed obligation set).
    fn dispatch_hash_of(&self, id: EffectId) -> Option<Hash> {
        self.log
            .iter()
            .find(|e| matches!(&e.body, EventBody::Dispatched { id: d, .. } if *d == id))
            .map(|e| e.hash())
    }

    /// The reducer continuation token that effect `id`'s `Dispatched` frame carried (§19b/§19e (B)):
    /// `Some(Some(token))` = a token was recorded, `Some(None)` = a token-free dispatch, `None` = no
    /// Dispatched frame for `id` (an invariant violation — every result has a prior dispatch). Derived
    /// from the DURABLE frame (the authoritative record), so it's replay-deterministic: the same result
    /// gets the same token whether built live or reconstructed on replay. This is how the token "rides
    /// the EffectResult" — [`record_result`] copies it onto the result event so a wasm reducer's fold can
    /// read it back as the guest's `resumes` without fold ever touching the log/map (fold stays pure).
    fn dispatch_token_of(&self, id: EffectId) -> Option<Option<Vec<u8>>> {
        // Scan from the END (rev): at most ONE matching frame per id, and a result/fire event is
        // near its dispatch/arm, so the reverse scan finds it fast — avoids an O(log^2) replay hot
        // path where a front scan re-walks the whole prefix for every EffectResult (PR#1253 review).
        self.log.iter().rev().find_map(|e| match &e.body {
            EventBody::Dispatched { id: d, token, .. } if *d == id => Some(token.clone()),
            _ => None,
        })
    }

    /// The effect KIND that dispatch `id`'s `Dispatched` frame recorded, or `None` if `id` has no
    /// `Dispatched` frame (e.g. a timer, opened by `TimerArmed`). Used on replay to tell a `Now`
    /// result apart (so `last_now` rebuilds only from `Now` results). Reads the durable frame, so it's
    /// replay-deterministic.
    fn dispatch_kind_of(&self, id: EffectId) -> Option<EffectKind> {
        // Scan from the END (rev): at most ONE matching frame per id, and a result/fire event is
        // near its dispatch/arm, so the reverse scan finds it fast — avoids an O(log^2) replay hot
        // path where a front scan re-walks the whole prefix for every EffectResult (PR#1253 review).
        self.log.iter().rev().find_map(|e| match &e.body {
            EventBody::Dispatched { id: d, kind, .. } if *d == id => Some(kind.clone()),
            _ => None,
        })
    }

    /// The reducer continuation token that timer `id`'s `TimerArmed` frame carried (§19e slice 2b-iii),
    /// the timer analogue of [`dispatch_token_of`]: `Some(Some(token))` = a token was armed, `Some(None)`
    /// = a token-free timer, `None` = no `TimerArmed` frame for `id`. Derived from the DURABLE arming
    /// frame so it's replay-deterministic — the same fire gets the same token live or reconstructed. This
    /// is how the token "rides the TimerFired": [`fire_due_timers`] copies it onto the fire event so a
    /// wasm reducer's fold reads it back as the guest's `resumes` without fold ever touching the log/map.
    fn timer_armed_token_of(&self, id: EffectId) -> Option<Option<Vec<u8>>> {
        // Scan from the END (rev): at most ONE matching frame per id, and a result/fire event is
        // near its dispatch/arm, so the reverse scan finds it fast — avoids an O(log^2) replay hot
        // path where a front scan re-walks the whole prefix for every EffectResult (PR#1253 review).
        self.log.iter().rev().find_map(|e| match &e.body {
            EventBody::TimerArmed { id: a, token, .. } if *a == id => Some(token.clone()),
            _ => None,
        })
    }

    /// Hash of the current tip (last log event) — the `cause` for effects its fold emits.
    fn tip_hash(&self) -> Hash {
        self.log.last().expect("log always has genesis").hash()
    }

    /// The ASYNC twin of [`Session::replay`] (operator all-async directive) — reconstruct a session from a
    /// persisted log, folding each observable event through a [`Reducer`] (`.await`). Identical
    /// reconstruction of the obligation sets / armed-timer table / `next_effect_id` / `last_now` high-water
    /// mark; only the re-fold awaits. Additive alongside the sync [`Session::replay`] during the migration;
    /// the sync one is removed once every reducer is `Reducer` (the operator's "one async trait only").
    ///
    /// Effects emitted during replay are IGNORED (§17 "replay re-folds with no live effect" — the results
    /// are already in the log), exactly as the sync path; so replayed-kv == live-kv (PR#990 finding #1).
    pub async fn replay_async(
        log: Vec<Event>,
        reducer: &dyn Reducer,
    ) -> Result<Session, KernelError> {
        match log.first().map(|e| &e.body) {
            Some(EventBody::Genesis { .. }) => {}
            _ => return Err(KernelError::MissingGenesis),
        }
        let mut s = Session {
            log: Vec::new(),
            kv: Kv::new(),
            next_effect_id: 0,
            settled: BTreeSet::new(),
            open: BTreeSet::new(),
            armed_timers: BTreeMap::new(),
            last_now: 0,
            store: None,
            persist_error: None,
        };
        for (i, event) in log.into_iter().enumerate() {
            if event.seq != i as u64 {
                return Err(KernelError::NonContiguousSeq {
                    expected: i as u64,
                    got: event.seq,
                });
            }
            // Reconstruct the obligation sets + armed-timer table + id counter from the log (§16c-S1/S5) —
            // IDENTICAL to the sync `replay`; only the re-fold below awaits.
            match &event.body {
                EventBody::Dispatched { id, .. } => {
                    s.open.insert(id.0);
                    s.next_effect_id = s.next_effect_id.max(id.0 + 1);
                }
                EventBody::TimerArmed {
                    id, deadline_ms, ..
                } => {
                    s.open.insert(id.0);
                    s.armed_timers.insert(id.0, *deadline_ms);
                    s.next_effect_id = s.next_effect_id.max(id.0 + 1);
                }
                EventBody::EffectResult { id, result, .. } => {
                    s.open.remove(&id.0);
                    s.settled.insert(id.0);
                    s.next_effect_id = s.next_effect_id.max(id.0 + 1);
                    if s.dispatch_kind_of(*id) == Some(EffectKind::Now) {
                        if let EffectOutcome::Ok(Some(crate::effect::Payload::Inline(bytes))) =
                            result
                        {
                            if let Ok(arr) = <[u8; 8]>::try_from(&bytes[..]) {
                                s.last_now = s.last_now.max(u64::from_le_bytes(arr));
                            }
                        }
                    }
                }
                EventBody::TimerFired { id, .. } => {
                    s.open.remove(&id.0);
                    s.armed_timers.remove(&id.0);
                    s.settled.insert(id.0);
                    s.next_effect_id = s.next_effect_id.max(id.0 + 1);
                }
                EventBody::AuthzDenied { id, .. } => {
                    s.next_effect_id = s.next_effect_id.max(id.0 + 1);
                }
                _ => {}
            }
            if observable(&event.body) {
                let _ = reducer.fold_async(&event, &mut s.kv).await;
            }
            s.log.push(event);
        }
        Ok(s)
    }

    /// The set of open (dispatched-but-unsettled) effect ids after recovery — what a driver must
    /// re-drive or time out (§16c-S1). Exposed for the recovery driver + tests.
    pub fn open_effect_ids(&self) -> Vec<EffectId> {
        self.open.iter().map(|n| EffectId(*n)).collect()
    }

    /// Boot a session from a persisted log on disk — the real recovery entry point (§16c-S1). Reads
    /// the durable log via [`crate::log_store::LogStore::recover`] and folds it through [`Session::replay`]
    /// to reconstruct KV + the open-obligation set, returning the session together with a
    /// [`RecoveryReport`] telling the driver what it must act on:
    ///
    /// - `kind` — how the log ended: `Clean`, `TornTail` (benign crash mid-append), or `Corrupt`
    ///   (a fully-present frame that didn't decode — an ALARM the driver must not miss; PR#993 #1
    ///   propagates this from `LogStore` so `Session::recover` callers can react, not just internal
    ///   code). The session is recovered to the last *whole* event before the tail either way.
    /// - `open_effects` — dispatched-but-unsettled effects the driver must re-drive (by their stable
    ///   idempotency key, so re-drive dedups rather than double-fires) or time out.
    ///
    /// Corruption is reported (not turned into a hard `Err`) so the driver keeps the recovered
    /// good-prefix + open-effects and DECIDES whether to proceed or halt — `report.kind` /
    /// `report.is_corrupt()` is the signal. An empty log (no file yet) is NOT recoverable as a session
    /// — the caller must `genesis()` a new one — reported as [`RecoverError::EmptyLog`].
    pub async fn recover(
        path: impl AsRef<std::path::Path>,
        reducer: &dyn Reducer,
    ) -> Result<(Session, RecoveryReport), RecoverError> {
        let recovered = crate::log_store::LogStore::recover(path).map_err(RecoverError::Io)?;
        if recovered.events.is_empty() {
            return Err(RecoverError::EmptyLog);
        }
        let kind = recovered.kind;
        let session = Session::replay_async(recovered.events, reducer)
            .await
            .map_err(RecoverError::Replay)?;
        let report = RecoveryReport {
            kind,
            open_effects: session.open_effect_ids(),
        };
        Ok((session, report))
    }
}

/// What [`Session::recover`] found that the driver must act on after booting from disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryReport {
    /// How the log ended (clean / torn tail / **corrupt**). `Corrupt` is an alarm the driver must not
    /// miss (PR#993 #1 — propagated from `LogStore` so `Session::recover` callers can react).
    pub kind: crate::log_store::RecoveryKind,
    /// Dispatched-but-unsettled effects (§16c-S1) the driver must re-drive by idempotency key or time
    /// out. Empty = the session crashed at a clean quiescent boundary, nothing in flight.
    pub open_effects: Vec<EffectId>,
}

impl RecoveryReport {
    /// Did recovery hit genuine corruption (an alarm, vs. a clean/torn end)?
    pub fn is_corrupt(&self) -> bool {
        self.kind == crate::log_store::RecoveryKind::Corrupt
    }
}

/// Failure booting a session from a persisted log.
#[derive(Debug)]
pub enum RecoverError {
    /// The log file couldn't be read.
    Io(std::io::Error),
    /// The log had no events (missing/empty file) — caller must `genesis()` a fresh session instead.
    EmptyLog,
    /// The recovered events didn't form a valid session log (no genesis / non-contiguous seq).
    Replay(KernelError),
}

/// A snapshot descriptor: `(seq, kv_root, reducer)` — the free per-event checkpoint (§4). A snapshot is
/// valid to fast-forward from only under a matching reducer (§7); v0 records the reducer so that check
/// is possible even though v0 doesn't yet prune history.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Snapshot {
    pub seq: u64,
    pub kv_root: Hash,
    pub reducer: Hash,
}

/// A structural, OUT-OF-BAND status report of a session — the CHEAP, non-interfering complement to a
/// fork-for-query (operator session-debug design, §4b tier-2 + tier-1). Assembled by reading the
/// session's ALREADY-MATERIALIZED state ([`Session::status_snapshot`]) — it appends NO event, runs NO
/// fold, and the session doesn't know it was asked, so it can never derail a session mid-work. Answers
/// "is X alive / stalled / idle?" for free (no model call); the semantic "what is X actually DOING?"
/// answer is the fork-for-query path (a fork's model summarizes itself). A supervisor or the concierge
/// reads this to route: cheap liveness here, spin a fork only when the semantic story is wanted.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StatusSnapshot {
    /// Derived liveness state (see [`SessionState`]).
    pub state: SessionState,
    /// Total events on the log — a coarse progress proxy.
    pub event_count: u64,
    /// The tip event's variant name (`"EffectResult"`, `"Dispatched"`, `"Inbound"`, …) — what it last did.
    pub last_event_kind: &'static str,
    /// Dispatched-but-unsettled effects (the §16c-S1 open-obligation set) — what the session is WAITING on.
    pub in_flight: Vec<InFlight>,
    /// Count of armed-but-unfired timers (§16c-S5).
    pub armed_timers: u32,
    /// The session's OWN published view: its KV entries under the `public/` prefix (a semantic status the
    /// session CHOSE to expose — §4b tier-1). The full KV is NOT here (higher-privilege access); only what
    /// the session published for observers. Empty if it published nothing (the structural fields still apply).
    pub published: std::collections::BTreeMap<Vec<u8>, Vec<u8>>,
}

/// A session's derived liveness state (§4b tier-2 — a structural fact, not something the session writes).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SessionState {
    /// Closed (a `Closed` event is on the log) — carries no more work.
    Closed,
    /// An in-flight effect has been outstanding longer than the stall threshold — LIKELY WEDGED (the
    /// wedge-detection triad as one structural fact: in-flight since T with now − T > threshold).
    Stalled,
    /// Has un-settled work (in-flight effects and/or armed timers) — actively doing something.
    Active,
    /// No in-flight effects, no armed timers, not closed — idle, awaiting input.
    Quiescent,
}

/// One dispatched-but-unsettled effect in a [`StatusSnapshot`] — what a session is blocked/waiting on.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct InFlight {
    /// The effect kind (`"Http"`, `"Model"`, `"Shell"`, …).
    pub kind: &'static str,
    /// The effect's resolved target (URL / model id / command) — what it's waiting on.
    pub target: String,
}

/// Does the reducer FOLD this event body? This defines the set of "observable" events; `replay`
/// consults it directly, and the live `drive` path folds the SAME set by construction — it only ever
/// calls `fold_tip` at append sites for observable events (Inbound tip, EffectResult, TimerFired,
/// AuthzDenied), and never folds the bookkeeping events (`Dispatched`/`TimerArmed`) it appends. So the
/// two paths fold identical event sets and live-kv can't diverge from replayed-kv (PR#990 finding #1);
/// this helper is the explicit encoding of that set on the replay side (PR#993 #2 — clarified that
/// `drive` enforces it structurally rather than calling this helper). A reducer observes its INPUTS and
/// OUTCOMES — inbound messages, effect results, timer fires, authorization denials (a denial is recovery
/// feedback, §9d) — but NOT the kernel's internal bookkeeping (`Dispatched`/`TimerArmed` exist only to
/// drive the crash-recovery obligation sets) nor `Genesis` (session setup, not a fold input).
/// The variant name of an event body, for a [`StatusSnapshot`]'s human-readable "last event kind" (a
/// debug label, not a wire tag — `event_ast`/`event` own the canonical encodings).
fn event_body_name(body: &EventBody) -> &'static str {
    match body {
        EventBody::Genesis { .. } => "Genesis",
        EventBody::Inbound { .. } => "Inbound",
        EventBody::Dispatched { .. } => "Dispatched",
        EventBody::EffectResult { .. } => "EffectResult",
        EventBody::TimerArmed { .. } => "TimerArmed",
        EventBody::TimerFired { .. } => "TimerFired",
        EventBody::FoldFailed { .. } => "FoldFailed",
        EventBody::AuthzDenied { .. } => "AuthzDenied",
        EventBody::Closed { .. } => "Closed",
    }
}

/// Clamp a `Now` effect's result to be strictly greater than `last_now` (monotonic clock, operator
/// ruling), updating `last_now` to the value handed back. The `Now` payload is a binary `u64` LE
/// nanoseconds-since-epoch reading; the executor produced it from the RAW wall clock (kernel stays
/// clock-free). If the raw reading `r <= last_now` (wall-clock resolution repeats, an NTP step back),
/// hand back `last_now + 1` instead — so successive `now()`s are strictly increasing and the log's time
/// ordering can't regress. The clamped value is what gets RECORDED, so replay re-derives the same
/// sequence. Only the `Ok(Some(Inline(8-byte u64)))` shape is clamped; any other outcome (Err/TimedOut,
/// a malformed non-8-byte payload) passes through untouched (defensive — never corrupt an outcome the
/// kernel can't interpret; a malformed Now reading is the executor's bug, surfaced as-is, not silently
/// rewritten). `last_now` still advances to the clamp floor even when the reading is used, via `max`.
///
/// OVERFLOW (PR#1253 review): the strictly-increasing floor is `last_now + 1`. At the astronomically
/// unreachable edge `last_now == u64::MAX` (year ~2554 in ns, AND the clock clamped up to there), there
/// is no larger value — `saturating_add` would return `u64::MAX` again, SILENTLY breaking the documented
/// strictly-increasing invariant. So we `checked_add(1)` and, on overflow, surface a fail-LOUD
/// `EffectOutcome::Err` (retry-classified `PERMANENT:` — the clock is genuinely exhausted, no retry
/// helps) rather than hand back a non-increasing value. Fail-loud on the impossible edge beats a silent
/// invariant break on a durable-clock path.
fn clamp_now_outcome(outcome: EffectOutcome, last_now: &mut u64) -> EffectOutcome {
    if let EffectOutcome::Ok(Some(crate::effect::Payload::Inline(bytes))) = &outcome {
        if let Ok(arr) = <[u8; 8]>::try_from(&bytes[..]) {
            let raw = u64::from_le_bytes(arr);
            let Some(floor) = last_now.checked_add(1) else {
                // last_now == u64::MAX: no strictly-greater value exists. Fail loud, don't regress.
                return EffectOutcome::Err(
                    "PERMANENT: monotonic clock exhausted (last_now at u64::MAX ns)".to_string(),
                );
            };
            let clamped = raw.max(floor);
            *last_now = clamped;
            return EffectOutcome::Ok(Some(crate::effect::Payload::Inline(
                clamped.to_le_bytes().to_vec().into(),
            )));
        }
    }
    outcome
}

/// The variant name of an effect kind, for a [`StatusSnapshot`]'s in-flight report (a debug label).
fn effect_kind_name(kind: &EffectKind) -> &'static str {
    match kind {
        EffectKind::Shell => "Shell",
        EffectKind::Http => "Http",
        EffectKind::Model => "Model",
        EffectKind::Now => "Now",
        EffectKind::Timer => "Timer",
        EffectKind::Emit => "Emit",
    }
}

fn observable(body: &EventBody) -> bool {
    match body {
        EventBody::Inbound { .. }
        | EventBody::EffectResult { .. }
        | EventBody::TimerFired { .. }
        | EventBody::AuthzDenied { .. }
        | EventBody::Closed { .. } => true,
        // FoldFailed is NOT folded (v0 records it for a supervisor to observe, but re-handing a failed
        // fold to the same failing reducer would recurse — a supervisor reacting is a later slice).
        EventBody::Genesis { .. }
        | EventBody::Dispatched { .. }
        | EventBody::TimerArmed { .. }
        | EventBody::FoldFailed { .. } => false,
    }
}

/// Derive a dispatch's idempotency key (§16c-S1). For v0 it's the hash of `(id, kind, target)` — stable
/// across a re-drive of the *same* dispatch, distinct across different effects. A real side-effecting
/// executor dedups on this so a crash-recovery re-drive doesn't double-apply.
fn idempotency_key_for(id: EffectId, req: &EffectRequest) -> Hash {
    let mut buf = Vec::new();
    buf.extend_from_slice(&id.0.to_le_bytes());
    buf.push(match req.kind {
        EffectKind::Shell => 0,
        EffectKind::Http => 1,
        EffectKind::Model => 2,
        EffectKind::Now => 3,
        EffectKind::Timer => 4,
        EffectKind::Emit => 5,
    });
    buf.extend_from_slice(req.target.as_bytes());
    Hash::of(&buf)
}

#[cfg(test)]
mod status_snapshot_tests {
    use super::*;
    use crate::authz::Authorizer;
    use crate::effect::{Capability, EffectKind, EffectRequest, ResourcePredicate, Timeliness};
    use crate::event::{ContentType, EventBody};
    use crate::executor::RecordingExecutor;
    use crate::reducer::{FoldOutput, Reducer};

    // A reducer that, on an inbound message, publishes a semantic status to `public/` and arms a Timer
    // (an open obligation that stays unsettled — no executor call — so the session reads as Active).
    struct StatusReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for StatusReducer {
        async fn fold_async(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    kv.put(b"public/status".to_vec(), b"investigating auth".to_vec());
                    kv.put(b"private/secret".to_vec(), b"nope".to_vec());
                    FoldOutput::with_effects(vec![crate::reducer::Effect {
                        request: EffectRequest::new(
                            EffectKind::Timer,
                            "1000", // absolute deadline ms
                            None,
                            Timeliness::Interactive,
                        ),
                        token: None,
                    }])
                }
                _ => FoldOutput::none(),
            }
        }
    }

    // A report-aware reducer (the fork-for-query summarize protocol, operator ruling (a)): on an ordinary
    // message it does work + publishes status; on the well-known `report` content-type it describes ITSELF
    // from LOCAL STATE (no model call — the operator's preferred path) by writing a summary to `public/`.
    // This is the generic-reducer shape a query fold uses: `if ct.is_report() { …summarize… }`.
    struct ReportingReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for ReportingReducer {
        async fn fold_async(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { content_type, .. } if content_type.is_report() => {
                    // Summarize from local KV alone — read the goal it recorded, describe progress.
                    let goal = kv
                        .get(b"private/goal")
                        .map(|v| String::from_utf8_lossy(v).into_owned())
                        .unwrap_or_else(|| "(no goal set)".to_string());
                    let summary = format!("working on: {goal}");
                    kv.put(b"public/summary".to_vec(), summary.into_bytes());
                    FoldOutput::none() // a local-state report takes NO effects (no model call)
                }
                EventBody::Inbound { .. } => {
                    // Ordinary work: record a private goal (what a real reducer would be doing).
                    kv.put(b"private/goal".to_vec(), b"the auth module".to_vec());
                    kv.put(b"public/status".to_vec(), b"active".to_vec());
                    FoldOutput::none()
                }
                _ => FoldOutput::none(),
            }
        }
    }

    fn report_inbound() -> EventBody {
        EventBody::Inbound {
            content_type: ContentType::report(),
            payload: crate::effect::Payload::Inline(b"summarize yourself".to_vec().into()),
        }
    }

    fn timer_cap() -> Authorizer {
        Authorizer::new(vec![Capability {
            kind: EffectKind::Timer,
            predicate: ResourcePredicate::Any,
        }])
    }

    fn inbound() -> EventBody {
        EventBody::Inbound {
            content_type: ContentType {
                family: "message".into(),
                version: 1,
            },
            payload: crate::effect::Payload::Inline(b"go".to_vec().into()),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fresh_session_is_quiescent_with_no_published_view() {
        let s = Session::genesis(Hash::of(b"r"));
        let snap = s.status_snapshot(Some(0), 300_000);
        assert_eq!(snap.state, SessionState::Quiescent);
        assert_eq!(snap.event_count, 1); // just genesis
        assert_eq!(snap.last_event_kind, "Genesis");
        assert!(snap.in_flight.is_empty());
        assert_eq!(snap.armed_timers, 0);
        assert!(snap.published.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn active_session_reports_armed_timer_and_only_the_public_kv() {
        let mut exec = RecordingExecutor::new();
        let mut s = Session::genesis(Hash::of(b"status-v1"));
        s.deliver_async(inbound(), None, &StatusReducer, &timer_cap(), &mut exec)
            .await
            .unwrap();

        let snap = s.status_snapshot(Some(500), 300_000); // now=500ms, well within the 5min threshold
                                                          // Active: the armed-but-unfired timer is un-settled work.
        assert_eq!(snap.state, SessionState::Active);
        assert_eq!(snap.armed_timers, 1);
        // The published view surfaces ONLY the `public/` key, NOT the private one (higher-privilege).
        assert_eq!(
            snap.published
                .get(b"public/status".as_slice())
                .map(|v| &v[..]),
            Some(&b"investigating auth"[..])
        );
        assert!(!snap.published.contains_key(b"private/secret".as_slice()));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fork_for_query_clones_state_without_touching_the_original() {
        let mut exec = RecordingExecutor::new();
        let mut s = Session::genesis(Hash::of(b"status-v1"));
        s.deliver_async(inbound(), None, &StatusReducer, &timer_cap(), &mut exec)
            .await
            .unwrap();

        // The parent is Active with one armed timer and its published status set.
        let parent_events_before = s.log().len();
        let parent_snap_before = s.status_snapshot(Some(500), 300_000);
        assert_eq!(parent_snap_before.state, SessionState::Active);
        assert_eq!(parent_snap_before.armed_timers, 1);

        // Fork it: the fork inherits the materialized KV (incl. the `public/` status) but starts as a
        // clean reactive session — its own genesis, NO inherited in-flight obligations or armed timers.
        let fork = s.fork_for_query();
        assert_eq!(fork.log().len(), 1); // just the fork's own genesis
        assert_eq!(fork.open_effects(), 0); // did NOT inherit the parent's open timer obligation
        assert_eq!(fork.next_timer_deadline(), None);
        // Same reducer-hash (folds identically) — the snapshot descriptor proves it.
        assert_eq!(fork.snapshot().reducer, s.snapshot().reducer);
        // The KV came across: the fork can read what the parent published.
        assert_eq!(
            fork.kv().get(b"public/status"),
            Some(&b"investigating auth"[..])
        );
        // And the private key too (the fork is a full-privilege clone of the materialized state; scoping
        // is the caller's capability concern, not the KV's — the fork just has the same KV).
        assert_eq!(fork.kv().get(b"private/secret"), Some(&b"nope"[..]));

        // NON-INTERFERENCE: forking read the parent immutably — its log, timers, and state are unchanged.
        assert_eq!(s.log().len(), parent_events_before);
        let parent_snap_after = s.status_snapshot(Some(500), 300_000);
        assert_eq!(parent_snap_after.state, SessionState::Active);
        assert_eq!(parent_snap_after.armed_timers, 1);
        assert_eq!(
            parent_snap_after.event_count,
            parent_snap_before.event_count
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn deliver_async_is_equivalent_to_sync_deliver() {
        // The safety property of the async migration: deliver_async must produce a session BYTE-IDENTICAL
        // to sync deliver for a sync-bodied reducer wrapped in SyncAsAsync (the async path only adds
        // cooperative yield; it changes no observable behavior). Build BOTH — a sync-delivered session and
        // an async-delivered one — and assert they match on the KV ROOT HASH (the whole KV, not one key),
        // the log length, and the derived status. A deliver/deliver_async drift fails loudly HERE.

        let mut sync_exec = RecordingExecutor::new();
        let mut sync_s = Session::genesis(Hash::of(b"status-v1"));
        sync_s
            .deliver_async(
                inbound(),
                None,
                &StatusReducer,
                &timer_cap(),
                &mut sync_exec,
            )
            .await
            .unwrap();

        let mut async_exec = RecordingExecutor::new();
        let mut async_s = Session::genesis(Hash::of(b"status-v1"));
        async_s
            .deliver_async(
                inbound(),
                None,
                &StatusReducer,
                &timer_cap(),
                &mut async_exec,
            )
            .await
            .unwrap();

        // EQUIVALENCE: identical KV root (whole-KV content-address), log length, and derived snapshot.
        assert_eq!(
            async_s.snapshot().kv_root,
            sync_s.snapshot().kv_root,
            "async KV root must equal sync KV root"
        );
        assert_eq!(async_s.log().len(), sync_s.log().len());
        let sync_snap = sync_s.status_snapshot(Some(500), 300_000);
        let async_snap = async_s.status_snapshot(Some(500), 300_000);
        assert_eq!(async_snap.state, sync_snap.state);
        assert_eq!(async_snap.armed_timers, sync_snap.armed_timers);
        assert_eq!(async_snap.published, sync_snap.published);
        // And the absolute expected result (not just "they agree"): Active, one armed timer, public-only.
        assert_eq!(async_snap.state, SessionState::Active);
        assert_eq!(async_snap.armed_timers, 1);
        assert_eq!(
            async_snap
                .published
                .get(b"public/status".as_slice())
                .map(|v| &v[..]),
            Some(&b"investigating auth"[..])
        );
        assert!(!async_snap
            .published
            .contains_key(b"private/secret".as_slice()));
    }

    // A reducer that arms a timer on an inbound message and, when that timer FIRES, publishes a marker —
    // so a test can prove fire_due_timers_async actually wakes the reducer (not just drains the table).
    struct TimerThenPublishReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for TimerThenPublishReducer {
        async fn fold_async(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    FoldOutput::with_effects(vec![crate::reducer::Effect {
                        request: EffectRequest::new(
                            EffectKind::Timer,
                            "1000", // absolute deadline ms
                            None,
                            Timeliness::Interactive,
                        ),
                        token: None,
                    }])
                }
                EventBody::TimerFired { .. } => {
                    kv.put(b"public/woke".to_vec(), b"timer-fired".to_vec());
                    FoldOutput::none()
                }
                _ => FoldOutput::none(),
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fire_due_timers_async_wakes_the_reducer_and_settles_the_timer() {
        // fire_due_timers_async is a new public API; pin it: arm a timer via an inbound, then fire it and
        // assert (a) it returns 1 (one timer fired), (b) the reducer WOKE (its TimerFired fold ran, writing
        // the marker), (c) the armed-timer + open-obligation sets drained (the timer settled). Same
        // determinism as the sync path — recorded fired_ms is the deadline, so replay is stable.
        let reducer = TimerThenPublishReducer;
        let mut exec = RecordingExecutor::new();
        let mut s = Session::genesis(Hash::of(b"timer-v1"));
        s.deliver_async(inbound(), None, &reducer, &timer_cap(), &mut exec)
            .await
            .unwrap();
        // Armed, not yet fired.
        assert_eq!(s.next_timer_deadline(), Some(1000));
        assert!(s.kv().get(b"public/woke").is_none());

        // Fire everything due at now=1500 (past the 1000ms deadline).
        let fired = s
            .fire_due_timers_async(1500, &reducer, &timer_cap(), &mut exec)
            .await;
        assert_eq!(fired, 1, "exactly one timer was due and fired");
        // The reducer woke on the TimerFired and published its marker.
        assert_eq!(s.kv().get(b"public/woke"), Some(&b"timer-fired"[..]));
        // The timer settled: no armed timers left, no open obligations.
        assert_eq!(s.next_timer_deadline(), None);
        assert_eq!(s.open_effects(), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fork_query_runs_a_summarize_fold_without_disturbing_the_parent() {
        // End-to-end shape of fork-for-query: fork, deliver a query message, the fork folds it (arming its
        // OWN timer here — a stand-in for the reducer's summarize work), and the parent is still untouched.
        let mut exec = RecordingExecutor::new();
        let mut s = Session::genesis(Hash::of(b"status-v1"));
        s.deliver_async(inbound(), None, &StatusReducer, &timer_cap(), &mut exec)
            .await
            .unwrap();
        let parent_events = s.log().len();

        let mut fork = s.fork_for_query();
        let mut fork_exec = RecordingExecutor::new();
        fork.deliver_async(
            inbound(),
            None,
            &StatusReducer,
            &timer_cap(),
            &mut fork_exec,
        )
        .await
        .unwrap();
        // The fork folded the query and did work in its OWN log.
        assert!(fork.log().len() > 1);
        assert_eq!(fork.status_snapshot(Some(0), 300_000).armed_timers, 1);

        // The parent's log length is untouched by anything the fork did.
        assert_eq!(s.log().len(), parent_events);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fork_query_summarizes_from_local_state_via_the_report_content_type() {
        // END-TO-END fork-for-query, all three landed pieces together (fork_for_query + the `report`
        // content-type + a report-aware reducer): a live session does work, then a DEBUG query forks it,
        // delivers a `report()` message, and the fork summarizes ITSELF from local state — with the
        // original session provably untouched (non-interference).
        let mut exec = RecordingExecutor::new();
        let mut live = Session::genesis(Hash::of(b"reporting-v1"));
        // The live session does ordinary work: records a private goal + public status.
        live.deliver_async(inbound(), None, &ReportingReducer, &timer_cap(), &mut exec)
            .await
            .unwrap();
        let live_events_before = live.log().len();
        assert_eq!(
            live.kv().get(b"private/goal"),
            Some(&b"the auth module"[..])
        );

        // Operator asks "what is this session doing?" → fork it and deliver a report query.
        let mut fork = live.fork_for_query();
        let mut fork_exec = RecordingExecutor::new();
        fork.deliver_async(
            report_inbound(),
            None,
            &ReportingReducer,
            &timer_cap(),
            &mut fork_exec,
        )
        .await
        .unwrap();

        // The fork summarized itself FROM LOCAL STATE — no effects at all. ASSERT it (not just narrate):
        // a local-state report must dispatch NOTHING (no model call, no world-action), so the fork's
        // executor is never invoked. Pinning this is the point — a regression that made a report query
        // perform effects (e.g. a model call slipping in, or a leaked world-action) would fail HERE, not
        // silently pass (PR#1324 review).
        assert!(
            fork_exec.seen.is_empty(),
            "a local-state report query must perform no effects, but the fork executor saw {}",
            fork_exec.seen.len()
        );

        // The fork summarized itself using the goal it inherited from the live session's materialized KV.
        let snap = fork.status_snapshot(Some(0), 300_000);
        assert_eq!(
            snap.published
                .get(b"public/summary".as_slice())
                .map(|v| &v[..]),
            Some(&b"working on: the auth module"[..])
        );

        // NON-INTERFERENCE: the live session never saw the query — its log is unchanged and it has no
        // `public/summary` (only the fork produced one).
        assert_eq!(live.log().len(), live_events_before);
        assert!(live.kv().get(b"public/summary").is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn closed_session_reports_closed() {
        let mut s = Session::genesis(Hash::of(b"r"));
        // Append a Closed event directly (a session that shut down).
        s.append(
            EventBody::Closed {
                outcome: crate::effect::Payload::Inline(b"".to_vec().into()),
            },
            None,
        );
        let snap = s.status_snapshot(Some(0), 300_000);
        assert_eq!(snap.state, SessionState::Closed);
        assert_eq!(snap.last_event_kind, "Closed");
    }
}

#[cfg(test)]
mod monotonic_now_tests {
    use super::*;
    use crate::authz::Authorizer;
    use crate::effect::{
        Capability, EffectKind, EffectRequest, Payload, ResourcePredicate, Timeliness,
    };
    use crate::event::{ContentType, EffectOutcome, EventBody};
    use crate::executor::RecordingExecutor;
    use crate::hash::Hash;
    use crate::kv::Kv;
    use crate::reducer::{Effect, FoldOutput, Reducer};

    // The clamp helper directly: a fresh reading above the floor passes through (and raises last_now);
    // a reading <= last_now is clamped UP to last_now+1 (strictly increasing).
    #[tokio::test(flavor = "current_thread")]
    async fn clamp_now_is_strictly_increasing() {
        let mut last = 0u64;
        let mk =
            |ns: u64| EffectOutcome::Ok(Some(Payload::Inline(ns.to_le_bytes().to_vec().into())));
        let read = |o: &EffectOutcome| match o {
            EffectOutcome::Ok(Some(Payload::Inline(b))) => {
                u64::from_le_bytes(<[u8; 8]>::try_from(&b[..]).unwrap())
            }
            _ => panic!("expected Now payload"),
        };
        // 1000 > 0 → passes through, last=1000.
        let a = clamp_now_outcome(mk(1000), &mut last);
        assert_eq!(read(&a), 1000);
        assert_eq!(last, 1000);
        // 1000 again (clock resolution repeat) → clamped to 1001.
        let b = clamp_now_outcome(mk(1000), &mut last);
        assert_eq!(read(&b), 1001);
        assert_eq!(last, 1001);
        // 500 (clock stepped BACKWARD, e.g. NTP) → clamped to 1002, never regresses.
        let c = clamp_now_outcome(mk(500), &mut last);
        assert_eq!(read(&c), 1002);
        assert_eq!(last, 1002);
        // 5000 (real advance) → passes through, last=5000.
        let d = clamp_now_outcome(mk(5000), &mut last);
        assert_eq!(read(&d), 5000);
        assert_eq!(last, 5000);
    }

    // OVERFLOW edge (PR#1253): at last_now == u64::MAX there's no strictly-greater value — clamp must
    // fail LOUD (a PERMANENT Err), not silently return u64::MAX again (which would break the invariant).
    #[tokio::test(flavor = "current_thread")]
    async fn clamp_now_at_u64_max_errors_instead_of_silently_repeating() {
        let mut last = u64::MAX;
        let reading = EffectOutcome::Ok(Some(Payload::Inline(
            u64::MAX.to_le_bytes().to_vec().into(),
        )));
        match clamp_now_outcome(reading, &mut last) {
            EffectOutcome::Err(msg) => {
                assert!(
                    msg.starts_with("PERMANENT:"),
                    "clock-exhausted is a PERMANENT error: {msg}"
                );
                assert!(msg.contains("monotonic clock exhausted"));
            }
            other => panic!("expected a fail-loud Err at u64::MAX, got {other:?}"),
        }
        // last_now is NOT advanced (there's nowhere to go) — it stays u64::MAX, not silently reused.
        assert_eq!(last, u64::MAX);
    }

    // A non-Now / malformed outcome passes through untouched (defensive — never corrupt an outcome the
    // kernel can't interpret as a u64-ns Now reading).
    #[tokio::test(flavor = "current_thread")]
    async fn clamp_now_passes_through_non_now_shapes() {
        let mut last = 42u64;
        // An Err passes through, last unchanged.
        let e = clamp_now_outcome(EffectOutcome::Err("boom".into()), &mut last);
        assert!(matches!(e, EffectOutcome::Err(_)));
        assert_eq!(last, 42);
        // A non-8-byte Inline payload (not a u64 ns) passes through untouched.
        let weird = EffectOutcome::Ok(Some(Payload::Inline(b"not-8".to_vec().into())));
        let out = clamp_now_outcome(weird, &mut last);
        assert!(matches!(out, EffectOutcome::Ok(Some(Payload::Inline(_)))));
        assert_eq!(last, 42);
    }

    // A reducer that requests a Now on every event, recording each returned ns into KV under a running
    // index so we can read the sequence back.
    struct NowReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for NowReducer {
        async fn fold_async(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } | EventBody::EffectResult { .. } => {
                    // On a Now result, stash it; then (up to 3 total) ask again to build a sequence.
                    if let EventBody::EffectResult {
                        result: EffectOutcome::Ok(Some(Payload::Inline(b))),
                        ..
                    } = &event.body
                    {
                        let n = kv.prefix_scan(b"t/").len();
                        kv.put(format!("t/{n}").into_bytes(), b.to_vec());
                        if n + 1 >= 3 {
                            return FoldOutput::none();
                        }
                    }
                    FoldOutput::with_effects(vec![Effect {
                        request: EffectRequest::new(
                            EffectKind::Now,
                            String::new(),
                            None,
                            Timeliness::Interactive,
                        ),
                        token: None,
                    }])
                }
                _ => FoldOutput::none(),
            }
        }
    }

    // An executor that returns the SAME raw ns for every Now (simulating a coarse clock) — the kernel's
    // clamp must still make the recorded sequence strictly increasing.
    struct StuckClock(u64);
    #[async_trait::async_trait(?Send)]
    impl Executor for StuckClock {
        async fn perform_async(&mut self, req: &EffectRequest, _key: Hash) -> EffectOutcome {
            assert_eq!(req.kind, EffectKind::Now);
            EffectOutcome::Ok(Some(Payload::Inline(self.0.to_le_bytes().to_vec().into())))
        }
    }

    fn now_cap() -> Authorizer {
        Authorizer::new(vec![Capability {
            kind: EffectKind::Now,
            predicate: ResourcePredicate::Any,
        }])
    }

    fn inbound() -> EventBody {
        EventBody::Inbound {
            content_type: ContentType {
                family: "message".into(),
                version: 1,
            },
            payload: Payload::Inline(b"go".to_vec().into()),
        }
    }

    fn recorded_now_sequence(s: &Session) -> Vec<u64> {
        s.kv()
            .prefix_scan(b"t/")
            .into_iter()
            .map(|(_, v)| u64::from_le_bytes(<[u8; 8]>::try_from(v).unwrap()))
            .collect()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn now_sequence_is_strictly_increasing_even_from_a_stuck_clock() {
        let mut exec = StuckClock(1000); // same raw reading every time
        let mut s = Session::genesis(Hash::of(b"now-v1"));
        s.deliver_async(inbound(), None, &NowReducer, &now_cap(), &mut exec)
            .await
            .unwrap();
        let seq = recorded_now_sequence(&s);
        assert_eq!(seq.len(), 3, "three Now readings recorded");
        // Despite the stuck 1000 clock, the kernel clamped to 1000, 1001, 1002 — strictly increasing.
        assert!(
            seq.windows(2).all(|w| w[1] > w[0]),
            "Now sequence must be strictly increasing, got {seq:?}"
        );
        assert_eq!(seq, vec![1000, 1001, 1002]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn replay_reconstructs_the_same_last_now_and_sequence() {
        let mut exec = StuckClock(1000);
        let mut s = Session::genesis(Hash::of(b"now-v1"));
        s.deliver_async(inbound(), None, &NowReducer, &now_cap(), &mut exec)
            .await
            .unwrap();
        let live_seq = recorded_now_sequence(&s);
        let live_last_now = s.last_now;

        // Replay the log: the recorded (already-clamped) Now results must rebuild the SAME last_now +
        // the SAME sequence — replay never re-clamps, it re-derives (determinism).
        let log = s.log().to_vec();
        let replayed = Session::replay_async(log, &NowReducer)
            .await
            .expect("replay");
        assert_eq!(recorded_now_sequence(&replayed), live_seq);
        assert_eq!(replayed.last_now, live_last_now);
        assert_eq!(replayed.last_now, 1002);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn replay_async_reconstructs_identically_to_sync_replay() {
        // The async twin `replay_async` (driving a Reducer) must reconstruct a session BYTE-IDENTICAL
        // to sync `replay` — the additive-migration invariant (replay's re-fold only awaits; it changes no
        // reconstruction). Build a session, then replay the SAME log both ways and compare KV root + last_now
        // + open set + log length. A replay/replay_async drift fails loudly HERE.
        let mut exec = StuckClock(1000);
        let mut s = Session::genesis(Hash::of(b"now-v1"));
        s.deliver_async(inbound(), None, &NowReducer, &now_cap(), &mut exec)
            .await
            .unwrap();
        let log = s.log().to_vec();

        let sync_replayed = Session::replay_async(log.clone(), &NowReducer)
            .await
            .expect("sync replay");
        let async_replayed = Session::replay_async(log, &NowReducer)
            .await
            .expect("async replay");

        assert_eq!(
            async_replayed.snapshot().kv_root,
            sync_replayed.snapshot().kv_root,
            "async replay KV root must equal sync replay KV root"
        );
        assert_eq!(async_replayed.last_now, sync_replayed.last_now);
        assert_eq!(async_replayed.last_now, 1002);
        assert_eq!(async_replayed.log().len(), sync_replayed.log().len());
        assert_eq!(
            recorded_now_sequence(&async_replayed),
            recorded_now_sequence(&sync_replayed)
        );
    }

    // A reducer that, on an inbound message, emits a `control/summary` effect carrying summary bytes in
    // its payload (the fork-for-query control-plane pattern). It's a control/* family → authz-exempt +
    // host-surfaced (register-by-string beat 3).
    struct SummaryEmitReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for SummaryEmitReducer {
        async fn fold_async(&self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    let mut request = EffectRequest::new(
                        EffectKind::Emit, // kind is irrelevant for a control family; the family drives it
                        "self",
                        Some(crate::effect::Payload::Inline(
                            b"i am investigating".to_vec().into(),
                        )),
                        Timeliness::Interactive,
                    );
                    // Control families carry the "control/" prefix; set it directly (a register-by-string
                    // caller would emit this family without any EffectKind at all).
                    request.content_type.family = crate::effect::effect_ct::SUMMARY.into();
                    FoldOutput::with_effects(vec![Effect {
                        request,
                        token: Some(b"cont-9".to_vec()),
                    }])
                }
                _ => FoldOutput::none(),
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_control_effect_is_surfaced_to_the_driver_not_authorized_or_routed() {
        // register-by-string beat 3: a control/* effect is authz-EXEMPT + NOT routed to the executor —
        // the kernel returns it to the driver via deliver_async_control. Prove all three: it's returned
        // (with its payload + token), the executor NEVER saw it, and a DENY-ALL authz did NOT deny it
        // (control is exempt — a normal effect under deny_all would be AuthzDenied).
        let mut exec = RecordingExecutor::new();
        let mut session = Session::genesis(Hash::of(b"control-v1"));
        let control = session
            .deliver_async_control(
                inbound(),
                None,
                &SummaryEmitReducer,
                &Authorizer::deny_all(),
                &mut exec,
            )
            .await
            .expect("deliver");

        // Surfaced to the driver: exactly the control/summary effect, payload + token intact.
        assert_eq!(control.len(), 1);
        assert_eq!(
            control[0].request.content_type.family.as_ref(),
            crate::effect::effect_ct::SUMMARY
        );
        assert_eq!(control[0].token.as_deref(), Some(&b"cont-9"[..]));
        match &control[0].request.payload {
            Some(crate::effect::Payload::Inline(b)) => assert_eq!(&b[..], b"i am investigating"),
            other => panic!("summary payload should carry the bytes, got {other:?}"),
        }
        // NEVER routed to the executor (control is host-answered, not a world-action).
        assert_eq!(exec.seen.len(), 0);
        // Authz-EXEMPT: deny_all did NOT produce an AuthzDenied for it (a normal effect would be denied).
        assert!(
            !session
                .log()
                .iter()
                .any(|e| matches!(e.body, EventBody::AuthzDenied { .. })),
            "a control/* effect must skip authz entirely — no AuthzDenied"
        );

        // The common deliver_async path drops the control Vec but is otherwise identical (returns ()).
        let mut exec2 = RecordingExecutor::new();
        let mut s2 = Session::genesis(Hash::of(b"control-v2"));
        s2.deliver_async(
            inbound(),
            None,
            &SummaryEmitReducer,
            &Authorizer::deny_all(),
            &mut exec2,
        )
        .await
        .expect("deliver (dropping control)");
        assert_eq!(exec2.seen.len(), 0);
    }

    // A reducer that, on one inbound message, emits BOTH a control/summary effect AND a regular effect/emit
    // effect — to prove the drive loop partitions WITHIN a single fold, not just when a turn is all-control.
    struct MixedEmitReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for MixedEmitReducer {
        async fn fold_async(&self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    // (1) a control/summary effect — must surface, authz-exempt, unrouted.
                    let mut control = EffectRequest::new(
                        EffectKind::Emit,
                        "self",
                        Some(crate::effect::Payload::Inline(b"summary".to_vec().into())),
                        Timeliness::Interactive,
                    );
                    control.content_type.family = crate::effect::effect_ct::SUMMARY.into();
                    // (2) a regular effect/emit effect — must authorize + route to the executor.
                    let regular = EffectRequest::new(
                        EffectKind::Emit,
                        "world",
                        Some(crate::effect::Payload::Inline(b"action".to_vec().into())),
                        Timeliness::Interactive,
                    );
                    FoldOutput::with_effects(vec![
                        Effect {
                            request: control,
                            token: Some(b"ctl".to_vec()),
                        },
                        Effect {
                            request: regular,
                            token: None,
                        },
                    ])
                }
                _ => FoldOutput::none(),
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_mixed_turn_splits_control_from_the_routed_effect() {
        // The partition discriminates PER-EFFECT within one fold: the control/summary effect surfaces to
        // the driver (authz-exempt, never routed), while its sibling effect/emit effect is authorized and
        // routed to the executor in the SAME turn. The single-control test can't catch a drive loop that
        // e.g. short-circuits the whole turn on the first control family; this pins the split.
        let mut exec = RecordingExecutor::new();
        let mut session = Session::genesis(Hash::of(b"mixed-v1"));
        // Grant the emit family (any resource) so the REGULAR effect authorizes; control is exempt anyway.
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Emit,
            predicate: crate::effect::ResourcePredicate::Any,
        }]);
        let control = session
            .deliver_async_control(inbound(), None, &MixedEmitReducer, &authz, &mut exec)
            .await
            .expect("deliver");

        // Control half: exactly the summary effect surfaced, with its token + payload intact.
        assert_eq!(control.len(), 1, "only the control/* effect surfaces");
        assert_eq!(
            control[0].request.content_type.family.as_ref(),
            crate::effect::effect_ct::SUMMARY
        );
        assert_eq!(control[0].token.as_deref(), Some(&b"ctl"[..]));
        // Payload bytes survive the partition intact — a drive-loop regression that drops/mutates the
        // surfaced control effect's payload during a mixed turn must fail here, not pass green (PR #1660
        // review: the comment claimed payload-intact but nothing asserted the bytes).
        match &control[0].request.payload {
            Some(crate::effect::Payload::Inline(b)) => assert_eq!(&b[..], b"summary"),
            other => {
                panic!("surfaced control effect should carry its payload bytes, got {other:?}")
            }
        }

        // Routed half: exactly the regular effect reached the executor (control never did).
        assert_eq!(exec.seen.len(), 1, "only the effect/* effect routes");
        assert_eq!(exec.seen[0].0.target.as_ref(), "world");
        assert_eq!(
            exec.seen[0].0.content_type.family.as_ref(),
            crate::effect::effect_ct::EMIT
        );

        // The regular effect was authorized (granted) → no AuthzDenied event for it, and it produced a
        // dispatch. (Control being exempt also produces no AuthzDenied — so zero denials total here.)
        assert!(
            !session
                .log()
                .iter()
                .any(|e| matches!(e.body, EventBody::AuthzDenied { .. })),
            "granted regular effect + exempt control → no AuthzDenied"
        );
    }

    // A reducer that, on an inbound message, emits a `control/capabilities` QUERY (no payload) — the guest
    // asking the kernel "what can I do?". I4: the kernel answers this INLINE (project + fold), not by
    // surfacing it to the driver.
    struct CapabilitiesQueryReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for CapabilitiesQueryReducer {
        async fn fold_async(&self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    let mut request =
                        EffectRequest::new(EffectKind::Emit, "self", None, Timeliness::Interactive);
                    request.content_type.family = crate::effect::effect_ct::CAPABILITIES.into();
                    FoldOutput::with_effects(vec![Effect {
                        request,
                        token: Some(b"cap-tok".to_vec()),
                    }])
                }
                _ => FoldOutput::none(),
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn control_capabilities_is_answered_inline_with_a_manifest_effect_result() {
        use crate::executor::CompositeExecutor;
        // I4: control/capabilities is KERNEL-answered inline — the kernel projects the capability manifest
        // and folds it back as an EffectResult, rather than surfacing it to the driver like other control/*.
        // Wire a CompositeExecutor that serves ONLY the emit family (mechanism dim), under a grant that
        // permits emit (policy dim) → the manifest must read emit=Granted and a NOT-served family=Absent.
        let mut exec = CompositeExecutor::new().with_effect(
            crate::effect::effect_ct::EMIT,
            Box::new(RecordingExecutor::new()),
        );
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Emit,
            predicate: crate::effect::ResourcePredicate::Any,
        }]);
        let mut session = Session::genesis(Hash::of(b"caps-v1"));
        let control = session
            .deliver_async_control(
                inbound(),
                None,
                &CapabilitiesQueryReducer,
                &authz,
                &mut exec,
            )
            .await
            .expect("deliver");

        // NOT surfaced to the driver (that's the whole point of inline-answer).
        assert!(
            control.is_empty(),
            "control/capabilities is answered inline, not surfaced"
        );

        // The kernel folded an EffectResult carrying the manifest bytes. Find it + decode.
        let payload = session
            .log()
            .iter()
            .find_map(|e| match &e.body {
                EventBody::EffectResult {
                    result: EffectOutcome::Ok(Some(crate::effect::Payload::Inline(b))),
                    ..
                } => Some(b.clone()),
                _ => None,
            })
            .expect("an inline capabilities EffectResult was folded");
        let a = cadenza_ast::codec::decode_detailed(&payload).expect("manifest payload decodes");
        let root = a
            .as_form(a.root, "capabilities-manifest")
            .expect("payload is a capabilities-manifest");
        let entries = a.as_form(root[1], "entries").expect("entries");
        // effect_ct::ALL families, one entry each; emit served+granted, a non-served one absent.
        assert_eq!(entries.len(), crate::effect::effect_ct::ALL.len());
        let grant_of = |fam: &str| {
            for &eid in entries {
                let e = a.as_form(eid, "entry").expect("entry");
                if a.as_str(e[0]) == Some(fam) {
                    // grant is the 2nd child: (granted)|(denied)|(absent) — return its head name.
                    for tag in ["granted", "denied", "absent"] {
                        if a.as_form(e[1], tag).is_some() {
                            return tag;
                        }
                    }
                }
            }
            "missing"
        };
        assert_eq!(
            grant_of(crate::effect::effect_ct::EMIT),
            "granted",
            "emit is served (mechanism) + permitted (policy) → granted"
        );
        assert_eq!(
            grant_of(crate::effect::effect_ct::SHELL),
            "absent",
            "shell is not served by this executor → absent (mechanism short-circuits)"
        );
    }
}
