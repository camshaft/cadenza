//! The agent HOST — assembles the kernel building blocks into a process that RUNS agents.
//!
//! This is the milestone the crate exists for: the kernel provides a `Session` (log + KV + the durable
//! dispatch/fold loop), a reducer, a `CompositeExecutor` that routes effects by kind, and an `Authorize`
//! gate. Individually those are library pieces. [`AgentHost`] is what *assembles* them into a live,
//! long-running host: it holds a **registry** of running sessions keyed by id, and for each one owns the
//! Session plus the reducer / authorizer / executor that drive it. Delivering an inbound event to a
//! session runs one full turn of the reactive loop — deliver → fold → authorize → dispatch (via the real
//! executors) → fold the result back — exactly the cycle an agent runs.
//!
//! A [`HostedSession`] bundles a `Session` with the three borrowed-at-`deliver`-time collaborators the
//! kernel loop needs (`&dyn Reducer`, `&dyn Authorize`, `&mut dyn Executor`), so the host owns them for
//! the session's lifetime and the registry can drive any session by id without the caller re-threading
//! them. This is the substrate the `session-status` query (a read over the registry) and later
//! fork-for-query build on.
//!
//! v0 is synchronous + single-threaded (the kernel loop is; §15b). The async/multi-session-scheduler
//! layer is a later slice that preserves this shape — a tokio task per session driving the same loop.

use cdz_kernel::authz::Authorize;
use cdz_kernel::event::EventBody;
use cdz_kernel::executor::CompositeExecutor;
use cdz_kernel::hash::Hash;
use cdz_kernel::kernel::{KernelError, Session};
use cdz_kernel::reducer::Reducer;
use std::collections::HashMap;

/// A session's identity in the host registry. A short opaque string the operator/driver assigns (e.g.
/// `"concierge"`, `"builder-42"`) — distinct from the kernel's per-effect `EffectId` and from the
/// content `Hash` of the reducer. Owned so the registry key needs no lifetime.
//
// The host drives sessions through the kernel's ASYNC loop (`Session::deliver_async`) so a long fold can
// cooperatively yield and sessions interleave (§15b). A reducer is therefore held as a `Box<dyn
// Reducer>` — the SINGLE reducer trait (operator "one async trait only"): a pure-Rust reducer writes
// a native `impl Reducer` (its `fold_async` runs to completion with no await point), and a wasm
// reducer uses `AsyncComponentReducer`. Both box directly as `Box<dyn Reducer>` — no wrapper.
#[derive(Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct SessionId(pub String);

impl SessionId {
    pub fn new(id: impl Into<String>) -> Self {
        SessionId(id.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One running agent: the kernel `Session` plus the collaborators that drive its loop. The host owns all
/// of them for the session's lifetime, so a registry can drive the session by id (the kernel's `deliver`
/// borrows reducer/authz/executor per call; bundling them here is what lets the host re-supply them).
pub struct HostedSession {
    session: Session,
    reducer: Box<dyn Reducer>,
    authz: Box<dyn Authorize>,
    executor: CompositeExecutor,
}

impl HostedSession {
    /// Start a fresh session from a genesis reducer hash, with its executor set + authorizer. The
    /// `reducer` drives folds; `executor` (a by-kind [`CompositeExecutor`]) performs authorized effects;
    /// `authz` gates them (SEC-F1). This is the assembly point — real executors (Now/Model/Http) go into
    /// `executor`, a real policy into `authz`.
    ///
    /// `reducer` is a `Box<dyn Reducer>`: a pure-Rust reducer is passed as
    /// `Box::new(my_reducer)`, a wasm reducer as `Box::new(AsyncComponentReducer::…)`.
    pub fn genesis(
        reducer_hash: Hash,
        reducer: Box<dyn Reducer>,
        authz: Box<dyn Authorize>,
        executor: CompositeExecutor,
    ) -> Self {
        HostedSession {
            session: Session::genesis(reducer_hash),
            reducer,
            authz,
            executor,
        }
    }

    /// Deliver one inbound event and run the reactive loop to quiescence (the kernel drives
    /// fold→dispatch→fold-result until no more effects are pending). This is one turn of the agent. Async
    /// so a long fold cooperatively yields and the host loop can interleave other sessions (§15b).
    pub async fn deliver(
        &mut self,
        body: EventBody,
        cause: Option<Hash>,
    ) -> Result<(), KernelError> {
        self.session
            .deliver_async(
                body,
                cause,
                &*self.reducer,
                &*self.authz,
                &mut self.executor,
            )
            .await
    }

    /// Fire every armed timer whose deadline has passed `now_ms`, waking the reducer (§9c). The host's
    /// scheduler calls this on a tick; returns how many fired.
    pub async fn fire_due_timers(&mut self, now_ms: u64) -> usize {
        self.session
            .fire_due_timers_async(now_ms, &*self.reducer, &*self.authz, &mut self.executor)
            .await
    }

    /// Read-only access to the underlying `Session` (for status queries, snapshotting, log inspection).
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// The earliest armed-timer deadline, if any — lets the host's scheduler know when to next tick.
    pub fn next_timer_deadline(&self) -> Option<u64> {
        self.session.next_timer_deadline()
    }

    /// How many effects are dispatched-but-unsettled (open obligations). Zero = the agent is idle,
    /// awaiting its next input.
    pub fn open_effects(&self) -> usize {
        self.session.open_effects()
    }

    /// FORK-FOR-QUERY (the semantic "what is this session DOING?" answer, §4b tier-1): non-interferingly
    /// ask a COPY of this session to summarize itself, WITHOUT touching the live session. The kernel's
    /// `Session::fork_for_query` clones this session's materialized KV + reducer-hash into a fresh
    /// EPHEMERAL session (clean id-space, no inherited obligations/timers/log, parent's `last_now` floor);
    /// this drives that fork with the caller-supplied collaborators, delivers a `report` event so a
    /// report-aware reducer summarizes itself, runs to quiescence, and returns the summary from the fork's
    /// `public/summary` KV — then DROPS the fork (never persisted). The parent is provably untouched (the
    /// fork is a separate `Session`; this method takes `&self`).
    ///
    /// The caller supplies the fork's `reducer` (the same logic the session runs — a `Box<dyn Reducer>`
    /// can't be cloned out of this `HostedSession`, so the caller re-provides it as a `&dyn Reducer`),
    /// a MODEL-ONLY `authz` (a scoped capability so a summarize-fold can call the model but CANNOT take
    /// world-actions — SEC-F1), and an `executor` to serve that model call. Returns `Some(summary_bytes)`
    /// if the reducer published `public/summary`, else `None` (it summarized elsewhere / didn't, or erred).
    pub async fn fork_for_query(
        &self,
        reducer: &dyn Reducer,
        authz: &dyn Authorize,
        executor: &mut CompositeExecutor,
    ) -> Option<Vec<u8>> {
        let mut fork = self.session.fork_for_query();
        // Deliver a `report` inbound so a report-aware reducer (branching on ct.is_report()) summarizes
        // itself from local KV. A KernelError here just means no summary (the fork is discarded regardless).
        let body = EventBody::Inbound {
            content_type: cdz_kernel::event::ContentType::report(),
            payload: cdz_kernel::effect::Payload::Inline(Vec::new().into()),
        };
        fork.deliver_async(body, None, reducer, authz, executor)
            .await
            .ok()?;
        // The summary the reducer published for observers (§4b tier-1). Cloned out before the fork drops.
        fork.kv().get(b"public/summary").map(|v| v.to_vec())
    }
}

/// The host: a registry of running agent sessions keyed by [`SessionId`]. Owns each [`HostedSession`],
/// routes inbound events to the right one, and is the object a `session-status <id>` query reads.
#[derive(Default)]
pub struct AgentHost {
    sessions: HashMap<SessionId, HostedSession>,
}

impl AgentHost {
    pub fn new() -> Self {
        AgentHost {
            sessions: HashMap::new(),
        }
    }

    /// Register a new running session under `id`. Returns the id back for convenience. If `id` already
    /// exists it is REPLACED (the caller chose to restart it) — the old session is dropped; a caller that
    /// wants collision-detection checks [`AgentHost::contains`] first.
    pub fn spawn(&mut self, id: SessionId, session: HostedSession) -> SessionId {
        self.sessions.insert(id.clone(), session);
        id
    }

    /// Is a session registered under this id?
    pub fn contains(&self, id: &SessionId) -> bool {
        self.sessions.contains_key(id)
    }

    /// Deliver an inbound event to the session `id`. `Ok(None)` means no such session (the caller can
    /// treat that as "unknown session"); `Ok(Some(Ok(())))` a successful turn; `Ok(Some(Err(_)))` a
    /// kernel error from the loop. Kept as a nested result so "unknown id" is distinct from "the loop
    /// erred" — a host serving many sessions must tell those apart.
    pub async fn deliver(
        &mut self,
        id: &SessionId,
        body: EventBody,
        cause: Option<Hash>,
    ) -> Option<Result<(), KernelError>> {
        match self.sessions.get_mut(id) {
            Some(s) => Some(s.deliver(body, cause).await),
            None => None,
        }
    }

    /// Read-only access to a hosted session (for a status query / inspection). `None` = unknown id.
    pub fn get(&self, id: &SessionId) -> Option<&HostedSession> {
        self.sessions.get(id)
    }

    /// The ids of all running sessions (for a "list sessions" surface), sorted for a deterministic
    /// listing.
    pub fn session_ids(&self) -> Vec<SessionId> {
        let mut ids: Vec<SessionId> = self.sessions.keys().cloned().collect();
        ids.sort();
        ids
    }

    /// Remove a finished/closed session from the registry, returning it if present (so a caller can
    /// inspect its final state). A completed agent is dropped from the host this way.
    pub fn remove(&mut self, id: &SessionId) -> Option<HostedSession> {
        self.sessions.remove(id)
    }

    /// How many sessions are registered.
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Is the registry empty (no running sessions)?
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Fire due timers across ALL registered sessions at `now_ms` (a host scheduler tick). Returns the
    /// total number of timers fired. A real async host wakes only sessions with a due deadline; v0's
    /// synchronous sweep is correct and simple.
    pub async fn fire_due_timers(&mut self, now_ms: u64) -> usize {
        let mut fired = 0;
        for s in self.sessions.values_mut() {
            fired += s.fire_due_timers(now_ms).await;
        }
        fired
    }

    /// The EARLIEST armed-timer deadline across all registered sessions, or `None` if no session has an
    /// armed timer. The async host loop uses this as its timer wheel — it sleeps until this deadline,
    /// then calls [`AgentHost::fire_due_timers`]. `None` means the loop only wakes on inbound events.
    pub fn next_timer_deadline_across_sessions(&self) -> Option<u64> {
        self.sessions
            .values()
            .filter_map(|s| s.next_timer_deadline())
            .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ClockExecutor;
    use cdz_kernel::authz::Authorizer;
    use cdz_kernel::effect::{
        effect_ct, Capability, EffectKind, EffectRequest, Payload, ResourcePredicate, Timeliness,
    };
    use cdz_kernel::event::{ContentType, EffectOutcome, Event};
    use cdz_kernel::kv::Kv;
    use cdz_kernel::reducer::{FoldOutput, Reducer};

    /// A tiny agent: on inbound "go" it asks the clock; when the time comes back it records "ran".
    struct ClockAgent;
    #[async_trait::async_trait(?Send)]
    impl Reducer for ClockAgent {
        async fn fold_async(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => FoldOutput::with(vec![EffectRequest::new(
                    EffectKind::Now,
                    String::new(),
                    None,
                    Timeliness::Interactive,
                )]),
                EventBody::EffectResult {
                    result: EffectOutcome::Ok(_),
                    ..
                } => {
                    kv.put(b"status".to_vec(), b"ran".to_vec());
                    FoldOutput::none()
                }
                _ => FoldOutput::none(),
            }
        }
    }

    fn inbound_go() -> EventBody {
        EventBody::Inbound {
            content_type: ContentType {
                family: "message".into(),
                version: 1,
            },
            payload: Payload::Inline(b"go".to_vec().into()),
        }
    }

    fn now_host() -> HostedSession {
        let executor =
            CompositeExecutor::new().with_effect(effect_ct::NOW, Box::new(ClockExecutor::new()));
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Now,
            predicate: ResourcePredicate::Any,
        }]);
        HostedSession::genesis(
            Hash::of(b"clock-agent-v1"),
            Box::new(ClockAgent),
            Box::new(authz),
            executor,
        )
    }

    /// An agent that arms a timer for `deadline_ms` on inbound "go", and records "woke" in KV when the
    /// timer FIRES (a `TimerFired` event) — so a test can prove the host's timer sweep actually woke it.
    struct TimerAgent {
        deadline_ms: u64,
    }
    #[async_trait::async_trait(?Send)]
    impl Reducer for TimerAgent {
        async fn fold_async(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => FoldOutput::with(vec![EffectRequest::new(
                    EffectKind::Timer,
                    self.deadline_ms.to_string(),
                    None,
                    Timeliness::Interactive,
                )]),
                EventBody::TimerFired { .. } => {
                    kv.put(b"woke".to_vec(), b"1".to_vec());
                    FoldOutput::none()
                }
                _ => FoldOutput::none(),
            }
        }
    }

    fn timer_host(deadline_ms: u64) -> HostedSession {
        // Timers are kernel-internal (no executor); the authorizer must permit Timer.
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Timer,
            predicate: ResourcePredicate::Any,
        }]);
        HostedSession::genesis(
            Hash::of(b"timer-agent-v1"),
            Box::new(TimerAgent { deadline_ms }),
            Box::new(authz),
            CompositeExecutor::new(),
        )
    }

    #[tokio::test]
    async fn host_spawns_and_drives_a_session_through_a_real_executor() {
        let mut host = AgentHost::new();
        let id = host.spawn(SessionId::new("agent-1"), now_host());
        assert!(host.contains(&id));
        assert_eq!(host.len(), 1);

        // Deliver an inbound event — the host drives the whole loop through the real ClockExecutor.
        let outcome = host.deliver(&id, inbound_go(), None).await;
        assert!(
            matches!(outcome, Some(Ok(()))),
            "a known session runs a turn"
        );

        // The agent ran to completion: it recorded "ran" and left nothing open.
        let hosted = host.get(&id).expect("session registered");
        assert_eq!(hosted.session().kv().get(b"status"), Some(&b"ran"[..]));
        assert_eq!(hosted.open_effects(), 0);
    }

    #[tokio::test]
    async fn delivering_to_an_unknown_session_is_none_not_a_panic() {
        let mut host = AgentHost::new();
        // No session registered → None (an unknown id is distinct from a loop error).
        assert!(host
            .deliver(&SessionId::new("nope"), inbound_go(), None)
            .await
            .is_none());
        assert!(host.get(&SessionId::new("nope")).is_none());
    }

    #[test]
    fn registry_lists_and_removes_sessions() {
        let mut host = AgentHost::new();
        host.spawn(SessionId::new("b"), now_host());
        host.spawn(SessionId::new("a"), now_host());
        // Listed sorted (deterministic).
        assert_eq!(
            host.session_ids(),
            vec![SessionId::new("a"), SessionId::new("b")]
        );
        // Remove one → gone.
        assert!(host.remove(&SessionId::new("a")).is_some());
        assert!(!host.contains(&SessionId::new("a")));
        assert_eq!(host.len(), 1);
        // Removing an absent id is None, not a panic.
        assert!(host.remove(&SessionId::new("a")).is_none());
    }

    #[tokio::test]
    async fn spawn_under_an_existing_id_replaces_the_session_restart_semantics() {
        // spawn() documents that re-spawning an existing id REPLACES the session (a restart — the old one
        // is dropped), not a no-op or a panic. A caller restarting a stuck agent relies on this. Drive the
        // first session to a known state, re-spawn a FRESH session under the same id, and assert the state
        // was reset (old dropped) + the registry didn't grow.
        let mut host = AgentHost::new();
        let id = SessionId::new("worker");
        host.spawn(id.clone(), now_host());
        // Drive the first instance to completion → it recorded "ran".
        host.deliver(&id, inbound_go(), None).await;
        assert_eq!(
            host.get(&id).unwrap().session().kv().get(b"status"),
            Some(&b"ran"[..])
        );
        assert_eq!(host.len(), 1);

        // Re-spawn a FRESH session under the SAME id (a restart). The old one is dropped, not kept.
        host.spawn(id.clone(), now_host());
        assert_eq!(
            host.len(),
            1,
            "replace, not add — the registry did not grow"
        );
        assert_eq!(
            host.get(&id).unwrap().session().kv().get(b"status"),
            None,
            "the replacement is a FRESH session — the prior 'ran' state was dropped"
        );
    }

    #[tokio::test]
    async fn two_sessions_run_independently() {
        let mut host = AgentHost::new();
        host.spawn(SessionId::new("a"), now_host());
        host.spawn(SessionId::new("b"), now_host());
        // Drive only "a".
        host.deliver(&SessionId::new("a"), inbound_go(), None).await;
        assert_eq!(
            host.get(&SessionId::new("a"))
                .unwrap()
                .session()
                .kv()
                .get(b"status"),
            Some(&b"ran"[..])
        );
        // "b" untouched — independent state.
        assert_eq!(
            host.get(&SessionId::new("b"))
                .unwrap()
                .session()
                .kv()
                .get(b"status"),
            None
        );
    }

    #[tokio::test]
    async fn hosted_session_fires_its_due_timer_on_a_tick() {
        // A HostedSession arms a timer on inbound; the host's fire_due_timers wakes it once the clock
        // reaches the deadline (the reactive-timer path, driven by the host's scheduler tick, not an
        // executor).
        let mut host = AgentHost::new();
        let id = SessionId::new("timed");
        host.spawn(id.clone(), timer_host(1000));
        host.deliver(&id, inbound_go(), None).await;

        // Armed but not yet fired: one open obligation (the timer), not yet woken.
        let hosted = host.get(&id).unwrap();
        assert_eq!(hosted.open_effects(), 1);
        assert_eq!(hosted.next_timer_deadline(), Some(1000));
        assert_eq!(hosted.session().kv().get(b"woke"), None);

        // A tick before the deadline fires nothing; a tick at the deadline fires it (wakes the reducer).
        assert_eq!(host.fire_due_timers(999).await, 0);
        assert_eq!(host.fire_due_timers(1000).await, 1);
        let hosted = host.get(&id).unwrap();
        assert_eq!(hosted.session().kv().get(b"woke"), Some(&b"1"[..]));
        assert_eq!(hosted.open_effects(), 0);
    }

    #[tokio::test]
    async fn host_fire_due_timers_sweeps_all_sessions_and_sums_fired() {
        // The all-session scheduler sweep: fire_due_timers(now) fires EVERY session's due timers and
        // returns the total count. Two sessions with different deadlines → a tick between them fires only
        // the earlier one; a later tick fires the other. A session with no timer contributes 0 (not woken).
        let mut host = AgentHost::new();
        host.spawn(SessionId::new("early"), timer_host(1000));
        host.spawn(SessionId::new("late"), timer_host(5000));
        host.spawn(SessionId::new("no-timer"), now_host()); // arms no timer
        host.deliver(&SessionId::new("early"), inbound_go(), None)
            .await;
        host.deliver(&SessionId::new("late"), inbound_go(), None)
            .await;
        // no-timer session gets no inbound → no armed timer.

        // Tick at 1000: only "early" is due → 1 fired total.
        assert_eq!(host.fire_due_timers(1000).await, 1);
        assert_eq!(
            host.get(&SessionId::new("early"))
                .unwrap()
                .session()
                .kv()
                .get(b"woke"),
            Some(&b"1"[..])
        );
        assert_eq!(
            host.get(&SessionId::new("late"))
                .unwrap()
                .session()
                .kv()
                .get(b"woke"),
            None,
            "the later timer has not fired yet"
        );

        // Tick at 5000: "late" now due (and "early" already fired) → 1 more fired.
        assert_eq!(host.fire_due_timers(5000).await, 1);
        assert_eq!(
            host.get(&SessionId::new("late"))
                .unwrap()
                .session()
                .kv()
                .get(b"woke"),
            Some(&b"1"[..])
        );
        // A further tick fires nothing (all timers settled).
        assert_eq!(host.fire_due_timers(9999).await, 0);
    }

    #[tokio::test]
    async fn next_deadline_across_sessions_is_the_min_and_none_when_no_timer_armed() {
        // The async host loop's timer wheel: `next_timer_deadline_across_sessions` is what the run-loop
        // sleeps until, so it must return the EARLIEST armed deadline across all sessions (min), and
        // `None` when nothing is armed (the loop then only wakes on inbound). Directly pinned here because
        // the loop consumes it but no test asserted its value.
        let mut host = AgentHost::new();
        // Empty registry → no timer → None.
        assert_eq!(host.next_timer_deadline_across_sessions(), None);

        host.spawn(SessionId::new("late"), timer_host(5000));
        host.spawn(SessionId::new("early"), timer_host(1000));
        host.spawn(SessionId::new("no-timer"), now_host()); // arms no timer

        // Before any inbound, no session has armed its timer yet → still None.
        assert_eq!(host.next_timer_deadline_across_sessions(), None);

        // Arm both timers (the no-timer session gets no inbound, so it contributes nothing).
        host.deliver(&SessionId::new("late"), inbound_go(), None)
            .await;
        host.deliver(&SessionId::new("early"), inbound_go(), None)
            .await;

        // The wheel returns the MIN of the two armed deadlines (1000), not 5000 and not the no-timer None.
        assert_eq!(host.next_timer_deadline_across_sessions(), Some(1000));

        // After the earliest fires, the wheel advances to the next-earliest (5000).
        assert_eq!(host.fire_due_timers(1000).await, 1);
        assert_eq!(host.next_timer_deadline_across_sessions(), Some(5000));

        // After the last fires, nothing armed → None again.
        assert_eq!(host.fire_due_timers(5000).await, 1);
        assert_eq!(host.next_timer_deadline_across_sessions(), None);
    }

    /// A report-aware agent: on a normal inbound it records live work in KV; on a `report` inbound it
    /// summarizes itself from that local KV into `public/summary` (no model call — the cheap tier-1 path).
    struct ReportingAgent;
    #[async_trait::async_trait(?Send)]
    impl Reducer for ReportingAgent {
        async fn fold_async(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { content_type, .. } if content_type.is_report() => {
                    // Summarize from local KV — here, echo the recorded phase into the published summary.
                    let phase = kv.get(b"phase").map(|v| v.to_vec()).unwrap_or_default();
                    let mut summary = b"phase=".to_vec();
                    summary.extend_from_slice(&phase);
                    kv.put(b"public/summary".to_vec(), summary);
                    FoldOutput::none()
                }
                EventBody::Inbound { .. } => {
                    kv.put(b"phase".to_vec(), b"working".to_vec());
                    FoldOutput::none()
                }
                _ => FoldOutput::none(),
            }
        }
    }

    #[tokio::test]
    async fn fork_for_query_summarizes_a_copy_without_touching_the_live_session() {
        // §4b tier-1: fork-for-query asks a COPY to summarize itself; the live session is untouched.
        let mut host = AgentHost::new();
        let id = SessionId::new("worker");
        host.spawn(
            id.clone(),
            HostedSession::genesis(
                Hash::of(b"reporting-v1"),
                Box::new(ReportingAgent),
                Box::new(Authorizer::deny_all()), // the live session takes no effects here
                CompositeExecutor::new(),
            ),
        );
        // Advance the live session so it has state to summarize (phase=working).
        host.deliver(&id, inbound_go(), None).await;
        let hosted = host.get(&id).unwrap();
        assert_eq!(hosted.session().kv().get(b"phase"), Some(&b"working"[..]));
        // Precondition: no summary on the LIVE session yet.
        assert_eq!(hosted.session().kv().get(b"public/summary"), None);
        let live_event_count = hosted.session().log().len();

        // Fork-for-query it: caller supplies the same (native Reducer) reducer + a model-only authz
        // (deny_all here — the summarize fold takes no effects) + an executor. Returns the published summary.
        let mut exec = CompositeExecutor::new();
        let summary = hosted
            .fork_for_query(&ReportingAgent, &Authorizer::deny_all(), &mut exec)
            .await;
        assert_eq!(
            summary.as_deref(),
            Some(&b"phase=working"[..]),
            "the fork summarizes the copied KV state"
        );

        // NON-INTERFERENCE: the live session is byte-for-byte unchanged — no summary appeared on it, and
        // its log didn't grow (the fork is a separate Session; fork_for_query took &self).
        let hosted = host.get(&id).unwrap();
        assert_eq!(hosted.session().kv().get(b"public/summary"), None);
        assert_eq!(hosted.session().log().len(), live_event_count);
    }
}
