//! The kernel core — `fold → authorize → durably-dispatch → execute → fold result` (§2).
//!
//! This is the v0.1 spine, single-session and in-memory, with the correctness-critical invariants from
//! the adversarial review designed in:
//!
//! - **S1 (durable dispatch):** before an effect is handed to an executor, a `Dispatched` event is
//!   appended to the authoritative log. Recovery re-drives un-resulted dispatches by idempotency key,
//!   so a crash between dispatch and result never double-fires or drops.
//! - **S4 (effect-id correlation + timeout-cancels):** each effect gets a monotonic `EffectId`; results
//!   fold back correlated by id. A timeout cancels the dispatch — once an outcome (Ok/Err/TimedOut) is
//!   recorded for an id, no second outcome for that id is ever accepted.
//! - **SEC-F1 (resource-scoped authz):** every effect is checked against a capability whose predicate
//!   gates the resolved target, not just the effect kind. A denied effect is logged, never executed.
//!
//! The KV is rebuilt by folding the log (it IS derived state — §4); a snapshot is `(seq, kv.root_hash,
//! reducer_hash)`. v0 keeps the log in memory; durable disk-backed storage is the next slice, but the
//! recovery *logic* (replay the log, re-drive open dispatches) is already exercised here via `replay`.

use crate::authz::Authorizer;
use crate::effect::{EffectId, EffectKind, EffectRequest};
use crate::event::{EffectOutcome, Event, EventBody};
use crate::executor::Executor;
use crate::hash::Hash;
use crate::kv::Kv;
use crate::reducer::Reducer;
use std::collections::{BTreeMap, BTreeSet};

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
        };
        s.log.push(Event {
            seq: 0,
            cause: None,
            body: EventBody::Genesis { reducer },
        });
        s
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

    /// The reducer this session was created with (from genesis). Panics only on a malformed log with
    /// no genesis, which `genesis()` makes impossible to construct.
    fn reducer_hash(&self) -> Hash {
        match self.log.first().map(|e| &e.body) {
            Some(EventBody::Genesis { reducer }) => *reducer,
            _ => Hash::of(b""),
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
        authz: &Authorizer,
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
        authz: &Authorizer,
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
    /// Append is INFALLIBLE in v0: pushing onto the in-memory log cannot fail, so it returns the new
    /// event's `Hash` directly (no `Result` to `let _ =`-swallow — the review flagged that pattern as a
    /// latent trap for when persistence lands). Wiring durable-append error handling INTO this path is a
    /// deliberate later slice; until then the type says what's true: it can't fail.
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
        self.log.push(event);
        hash
    }

    /// Run the reducer over the just-appended tip and process the effects it emits until quiescent.
    ///
    /// Causal DAG (§5): every effect is `cause`-linked to the event that unlocked it — the reducer
    /// output of folding event E is caused by E. So the chain threads
    /// trigger → dispatch → result → (next dispatch caused by that result) → …, which is exactly the
    /// provenance audit / blast-radius (§9f) / on-behalf-of (§12f) traversals need.
    fn drive(&mut self, reducer: &dyn Reducer, authz: &Authorizer, executor: &mut dyn Executor) {
        // Worklist of (request, cause) — cause is the hash of the event whose fold emitted the request.
        // The initial batch is caused by the just-appended tip.
        let trigger = self.tip_hash();
        let mut to_process = self.fold_tip(reducer, trigger);

        while let Some((req, cause)) = to_process.pop() {
            let id = EffectId(self.next_effect_id);
            self.next_effect_id += 1;

            // SEC-F1: authorize against the resolved target, not just the kind. The denial is caused
            // by the event that requested the effect.
            if let Err(reason) = authz.authorize(&req) {
                self.append(EventBody::AuthzDenied { id, reason }, Some(cause));
                // Denied effect never reaches an executor; folding the denial may prompt the reducer to
                // recover (handled on the next inbound; v0 doesn't re-fold denials into `drive`).
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
                        // than panicking (totality, §17). It settles the id so nothing waits on it.
                        self.append(
                            EventBody::AuthzDenied {
                                id,
                                reason: format!("timer deadline not a u64 ms: {:?}", req.target),
                            },
                            Some(cause),
                        );
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
                },
                Some(cause),
            );

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
    fn fold_tip(&mut self, reducer: &dyn Reducer, cause: Hash) -> Vec<(EffectRequest, Hash)> {
        let tip = self.log.last().expect("log always has genesis").clone();
        let out = reducer.fold(&tip, &mut self.kv);
        let mut v: Vec<(EffectRequest, Hash)> =
            out.effects.into_iter().map(|r| (r, cause)).collect();
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
    ) -> Vec<(EffectRequest, Hash)> {
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
            // Re-fold non-genesis events to rebuild KV (effects emitted during replay are IGNORED —
            // their results are already in the log; §17 "replay re-folds with no live effect").
            if !matches!(event.body, EventBody::Genesis { .. }) {
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
    /// - `torn_tail` — the log ended in an interrupted final write (benign — the crash point); the
    ///   session is recovered to the last *whole* event before it.
    /// - `open_effects` — dispatched-but-unsettled effects the driver must re-drive (by their stable
    ///   idempotency key, so re-drive dedups rather than double-fires) or time out.
    ///
    /// An empty log (no file yet) is NOT recoverable as a session — the caller must `genesis()` a new
    /// one. That case is reported as [`RecoverError::EmptyLog`] so it's an explicit branch, not a
    /// silent empty session.
    pub fn recover(
        path: impl AsRef<std::path::Path>,
        reducer: &dyn Reducer,
    ) -> Result<(Session, RecoveryReport), RecoverError> {
        let recovered = crate::log_store::LogStore::recover(path).map_err(RecoverError::Io)?;
        if recovered.events.is_empty() {
            return Err(RecoverError::EmptyLog);
        }
        let session = Session::replay(recovered.events, reducer).map_err(RecoverError::Replay)?;
        let report = RecoveryReport {
            torn_tail: recovered.torn_tail,
            open_effects: session.open_effect_ids(),
        };
        Ok((session, report))
    }
}

/// What [`Session::recover`] found that the driver must act on after booting from disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryReport {
    /// The log ended in a torn (interrupted) final write — the crash point. Benign; the recovered
    /// session stops at the last whole event.
    pub torn_tail: bool,
    /// Dispatched-but-unsettled effects (§16c-S1) the driver must re-drive by idempotency key or time
    /// out. Empty = the session crashed at a clean quiescent boundary, nothing in flight.
    pub open_effects: Vec<EffectId>,
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
