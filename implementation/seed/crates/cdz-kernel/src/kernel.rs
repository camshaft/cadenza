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
    /// The durable log this session writes THROUGH as it appends (§16c-S1), if attached via
    /// [`Session::attach_log`]. When present, every appended event is persisted (append + flush) before
    /// `append` returns — so the S1 "Dispatched durable before its effect routes" ordering is enforced
    /// IN-KERNEL, not left to a driver mirroring events by hand. `None` = an in-memory-only session
    /// (tests, or a caller that persists separately). Persistence lives here, next to the in-memory log
    /// it shadows, so the two never diverge.
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

    /// Deliver an inbound event and run the fold→dispatch cycle to quiescence: fold the event, perform
    /// each requested (and authorized) effect, fold its result, repeat until no new effects. This is
    /// the reactive step (§9d): appending the inbound event is what drives the reducer.
    pub fn deliver(
        &mut self,
        body: EventBody,
        cause: Option<Hash>,
        reducer: &dyn Reducer,
        authz: &dyn Authorize,
        executor: &mut dyn Executor,
    ) -> Result<(), KernelError> {
        self.append(body, cause);
        self.drive(reducer, authz, executor);
        Ok(())
    }

    /// Fire every armed timer whose absolute deadline has passed `now_ms`, injecting a `TimerFired`
    /// (§9c) and driving the reducer for each — the kernel, not an executor, is what wakes a timer. The
    /// driver calls this with the current wall clock (e.g. on a scheduler tick); the reducer stays
    /// clock-free because it only ever sees the recorded `fired_ms`. Returns how many fired.
    ///
    /// Determinism (§9c): the FIRED time recorded is the timer's own `deadline_ms`, not `now_ms` — so a
    /// timer that fires 5ms or 5s late still records the same frozen fact, and replay reconstructs
    /// identically regardless of when `fire_due_timers` happened to run. Fires in deadline order so a
    /// batch of overdue timers wakes the reducer oldest-first.
    pub fn fire_due_timers(
        &mut self,
        now_ms: u64,
        reducer: &dyn Reducer,
        authz: &dyn Authorize,
        executor: &mut dyn Executor,
    ) -> usize {
        // Collect due (id, deadline) in deadline order; drain from the table happens in `append` when
        // the `TimerFired` lands.
        let mut due: Vec<(u64, u64)> = self
            .armed_timers
            .iter()
            .filter(|(_, &deadline)| deadline <= now_ms)
            .map(|(&id, &deadline)| (deadline, id))
            .collect();
        due.sort_unstable();
        for (deadline, id) in &due {
            self.append(
                EventBody::TimerFired {
                    id: EffectId(*id),
                    fired_ms: *deadline,
                },
                None,
            );
            // Firing wakes the reducer (a timer is a reactive trigger, §9d) — drive to quiescence,
            // which may itself arm more timers or dispatch effects.
            self.drive(reducer, authz, executor);
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
            EventBody::TimerArmed { id, deadline_ms } => {
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

    /// Run the reducer over the just-appended tip and process the effects it emits until quiescent.
    ///
    /// Causal DAG (§5): every effect is `cause`-linked to the event that unlocked it — the reducer
    /// output of folding event E is caused by E. So the chain threads
    /// trigger → dispatch → result → (next dispatch caused by that result) → …, which is exactly the
    /// provenance audit / blast-radius (§9f) / on-behalf-of (§12f) traversals need.
    fn drive(&mut self, reducer: &dyn Reducer, authz: &dyn Authorize, executor: &mut dyn Executor) {
        // Worklist of (request, cause) — cause is the hash of the event whose fold emitted the request.
        // The initial batch is caused by the just-appended tip.
        let trigger = self.tip_hash();
        let initial = self.fold_tip(reducer, trigger);
        self.drive_worklist(initial, reducer, authz, executor);
    }

    /// Process a worklist of `(request, cause)` effects to quiescence — the shared core of `drive` (which
    /// seeds it by folding the tip) and `time_out_effect` (which seeds it with a timeout continuation's
    /// effects). Kept as one method so every entry point runs the IDENTICAL authorize → durable-dispatch
    /// → execute → fold-result loop (no second, drifting copy).
    fn drive_worklist(
        &mut self,
        mut to_process: Vec<(Effect, Hash)>,
        reducer: &dyn Reducer,
        authz: &dyn Authorize,
        executor: &mut dyn Executor,
    ) {
        while let Some((effect, cause)) = to_process.pop() {
            let Effect {
                request: req,
                token,
            } = effect;
            let id = EffectId(self.next_effect_id);
            self.next_effect_id += 1;

            // SEC-F1: authorize against the resolved target, not just the kind. The denial is caused
            // by the event that requested the effect.
            if let Err(reason) = authz.authorize(&req) {
                // A denial is an OBSERVABLE outcome (§9d recovery): the reducer folds it in BOTH the
                // live and replay paths, so live-kv == replayed-kv (PR#990 finding #1 — the denial was
                // appended but not folded live, while replay folds it → divergence). Folding may emit
                // recovery effects, which join the worklist.
                let denial_hash = self.append(EventBody::AuthzDenied { id, reason }, Some(cause));
                for pair in self.fold_tip(reducer, denial_hash) {
                    to_process.push(pair);
                }
                continue;
            }

            // Timers are NOT executor calls (§9c): a `Timer` effect arms a deadline the KERNEL fires
            // later (`fire_due_timers`), keeping the reducer clock-free. Its target is the absolute
            // wall-clock deadline in ms (§16c-S5). Arm it (an open obligation) and move on — no
            // executor, no synchronous result.
            if req.kind == EffectKind::Timer {
                match req.target.parse::<u64>() {
                    Ok(deadline_ms) => {
                        self.append(EventBody::TimerArmed { id, deadline_ms }, Some(cause));
                    }
                    Err(_) => {
                        // A malformed deadline is a request error, surfaced like a denial (audit) rather
                        // than panicking (totality, §17). Observable → folded in both paths (finding #1).
                        let denial_hash = self.append(
                            EventBody::AuthzDenied {
                                id,
                                reason: format!("timer deadline not a u64 ms: {:?}", req.target),
                            },
                            Some(cause),
                        );
                        for pair in self.fold_tip(reducer, denial_hash) {
                            to_process.push(pair);
                        }
                    }
                }
                continue;
            }

            let idempotency_key = idempotency_key_for(id, &req);

            // S1: append the DURABLE dispatch record BEFORE routing to the executor. Caused by the
            // requesting event; its result (below) is in turn caused by the dispatch.
            let dispatch_hash = self.append(
                EventBody::Dispatched {
                    id,
                    kind: req.kind.clone(),
                    target: req.target.clone(),
                    idempotency_key,
                    deadline_ms: None,
                    // Thread the reducer's continuation token (§19e) into the durable frame: `None` for a
                    // Rust reducer (correlates by EffectId), the guest's `correlation` for a wasm
                    // `ComponentReducer`. Recording it here is what lets recovery rebuild the
                    // EffectId↔token map from the log (slice-1 guard).
                    token: token.clone(),
                },
                Some(cause),
            );

            // S1 latch-check BEFORE routing (concierge ruling, tier B): if persisting THIS Dispatched
            // frame failed, the dispatch is NOT durable — so we must NOT route the effect, because an
            // executor may perform an irreversible external side-effect (e.g. ShellExecutor spawns a
            // process) that a crash-recovery would then re-drive with no record it already ran. Instead
            // of performing, record the effect as failed-undurable (an observable outcome the reducer
            // folds, live == replay) and move on. The latch stays set (surfaced to the driver via
            // take_persist_error); subsequent dispatches short-circuit here too. This is the tier-B
            // durable-before-route guarantee at the actual danger point (an un-doable route on an
            // un-durable dispatch); tier A (strict fallible-abort of the whole drive) is the tracked
            // hardening for when external routing goes live in anger.
            if self.persist_error.is_some() {
                let outcome = EffectOutcome::Err(
                    "dispatch not durably logged (persist failure) — effect NOT routed (S1)"
                        .to_string(),
                );
                let more = self.record_result(id, outcome, reducer, dispatch_hash);
                for pair in more {
                    to_process.push(pair);
                }
                continue;
            }

            // Route + execute. (In v0 this is synchronous; the async path preserves the ordering: the
            // Dispatched record is already durable, so a crash here recovers via replay.)
            let outcome = executor.perform(&req, idempotency_key);

            // Fold the result back (S4: correlated by id), caused by its dispatch. Any further effects
            // the reducer emits when it folds the result are caused by the RESULT event (the new tip).
            let more = self.record_result(id, outcome, reducer, dispatch_hash);
            for pair in more {
                to_process.push(pair);
            }
        }
    }

    /// Fold the current tip event through the reducer, applying its KV writes and returning its
    /// requested effects each paired with `cause` (the tip's hash — what unlocked them). Reversed so a
    /// `pop`-driven worklist yields them in emission order.
    fn fold_tip(&mut self, reducer: &dyn Reducer, cause: Hash) -> Vec<(Effect, Hash)> {
        let tip = self.log.last().expect("log always has genesis").clone();
        let out = reducer.fold(&tip, &mut self.kv);
        let mut v: Vec<(Effect, Hash)> = out.effects.into_iter().map(|e| (e, cause)).collect();
        v.reverse();
        v
    }

    /// Record an effect result, honoring timeout-cancels (§16c-S4): a result for an already-settled id
    /// is dropped. The result event is `cause`-linked to its dispatch (`dispatch_hash`), and any
    /// further effects the reducer emits folding the result are caused by the result event itself.
    fn record_result(
        &mut self,
        id: EffectId,
        outcome: EffectOutcome,
        reducer: &dyn Reducer,
        dispatch_hash: Hash,
    ) -> Vec<(Effect, Hash)> {
        if self.settled.contains(&id.0) {
            // Already settled (e.g. timed out earlier) — drop the late result. No double-resume.
            return Vec::new();
        }
        let result_hash = self.append(
            EventBody::EffectResult {
                id,
                result: outcome,
            },
            Some(dispatch_hash),
        );
        self.fold_tip(reducer, result_hash)
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
    pub fn time_out_effect(
        &mut self,
        id: EffectId,
        reducer: &dyn Reducer,
        authz: &dyn Authorize,
        executor: &mut dyn Executor,
    ) -> bool {
        // Idempotent: only an OPEN id can be timed out. Settled (or never-dispatched) → no-op, so a late
        // real result and a timeout can't both settle one id (§16c-S4 at-most-once).
        if !self.open.contains(&id.0) {
            return false;
        }
        // `open` holds BOTH dispatched-effect ids AND armed-timer ids — but only a DISPATCHED effect can
        // be timed out (a timer isn't a hung external call; it fires via `fire_due_timers`). A timer id
        // has no `Dispatched` event, so `dispatch_hash_of` is None → return false (Copilot PR#1016: the
        // old code panicked here, contradicting the "never dispatched → false" contract). Timing out a
        // timer is a no-op, not a crash.
        let Some(dispatch_hash) = self.dispatch_hash_of(id) else {
            return false;
        };
        // Link the timeout result to the dispatch that opened it (causal DAG §5), like a real result.
        let more = self.record_result(id, EffectOutcome::TimedOut, reducer, dispatch_hash);
        // The reducer's timeout continuation may emit further effects — drive them to quiescence.
        self.drive_worklist(more, reducer, authz, executor);
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

    /// Hash of the current tip (last log event) — the `cause` for effects its fold emits.
    fn tip_hash(&self) -> Hash {
        self.log.last().expect("log always has genesis").hash()
    }

    /// Replay a log to reconstruct KV + counters + the open-obligation set (§16c-S1 recovery). This is
    /// the recovery path: given a persisted log, rebuild derived state deterministically. Effects are
    /// NOT re-executed during replay (§17) — only their recorded results are re-folded.
    pub fn replay(log: Vec<Event>, reducer: &dyn Reducer) -> Result<Session, KernelError> {
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
            // Reconstruct the obligation sets + armed-timer table + id counter from the log (§16c-S1/S5).
            match &event.body {
                EventBody::Dispatched { id, .. } => {
                    s.open.insert(id.0);
                    s.next_effect_id = s.next_effect_id.max(id.0 + 1);
                }
                EventBody::TimerArmed { id, deadline_ms } => {
                    s.open.insert(id.0);
                    s.armed_timers.insert(id.0, *deadline_ms);
                    s.next_effect_id = s.next_effect_id.max(id.0 + 1);
                }
                EventBody::EffectResult { id, .. } => {
                    s.open.remove(&id.0);
                    s.settled.insert(id.0);
                    s.next_effect_id = s.next_effect_id.max(id.0 + 1);
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
            // Re-fold OBSERVABLE events to rebuild KV — the SAME set the live `drive` path folds, so
            // replayed-kv == live-kv (PR#990 finding #1). Kernel-internal bookkeeping events
            // (Dispatched, TimerArmed) are NOT folded in either path; effects emitted during replay are
            // IGNORED — their results are already in the log (§17 "replay re-folds with no live effect").
            if observable(&event.body) {
                let _ = reducer.fold(&event, &mut s.kv);
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
    pub fn recover(
        path: impl AsRef<std::path::Path>,
        reducer: &dyn Reducer,
    ) -> Result<(Session, RecoveryReport), RecoverError> {
        let recovered = crate::log_store::LogStore::recover(path).map_err(RecoverError::Io)?;
        if recovered.events.is_empty() {
            return Err(RecoverError::EmptyLog);
        }
        let kind = recovered.kind;
        let session = Session::replay(recovered.events, reducer).map_err(RecoverError::Replay)?;
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
fn observable(body: &EventBody) -> bool {
    match body {
        EventBody::Inbound { .. }
        | EventBody::EffectResult { .. }
        | EventBody::TimerFired { .. }
        | EventBody::AuthzDenied { .. }
        | EventBody::Closed { .. } => true,
        EventBody::Genesis { .. } | EventBody::Dispatched { .. } | EventBody::TimerArmed { .. } => {
            false
        }
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
