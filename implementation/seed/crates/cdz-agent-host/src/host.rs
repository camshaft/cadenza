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
//! fork-query build on.
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
    /// fold→dispatch→fold-result until no more effects are pending). This is one turn of the agent.
    pub fn deliver(&mut self, body: EventBody, cause: Option<Hash>) -> Result<(), KernelError> {
        self.session.deliver(
            body,
            cause,
            &*self.reducer,
            &*self.authz,
            &mut self.executor,
        )
    }

    /// Fire every armed timer whose deadline has passed `now_ms`, waking the reducer (§9c). The host's
    /// scheduler calls this on a tick; returns how many fired.
    pub fn fire_due_timers(&mut self, now_ms: u64) -> usize {
        self.session
            .fire_due_timers(now_ms, &*self.reducer, &*self.authz, &mut self.executor)
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
    pub fn deliver(
        &mut self,
        id: &SessionId,
        body: EventBody,
        cause: Option<Hash>,
    ) -> Option<Result<(), KernelError>> {
        self.sessions.get_mut(id).map(|s| s.deliver(body, cause))
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
    pub fn fire_due_timers(&mut self, now_ms: u64) -> usize {
        self.sessions
            .values_mut()
            .map(|s| s.fire_due_timers(now_ms))
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ClockExecutor;
    use cdz_kernel::authz::Authorizer;
    use cdz_kernel::effect::{
        Capability, EffectKind, EffectRequest, Payload, ResourcePredicate, Timeliness,
    };
    use cdz_kernel::event::{ContentType, EffectOutcome, Event};
    use cdz_kernel::kv::Kv;
    use cdz_kernel::reducer::{FoldOutput, Reducer};

    /// A tiny agent: on inbound "go" it asks the clock; when the time comes back it records "ran".
    struct ClockAgent;
    impl Reducer for ClockAgent {
        fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => FoldOutput::with(vec![EffectRequest {
                    kind: EffectKind::Now,
                    target: String::new(),
                    payload: None,
                    timeliness: Timeliness::Interactive,
                }]),
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
            CompositeExecutor::new().with(EffectKind::Now, Box::new(ClockExecutor::new()));
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

    #[test]
    fn host_spawns_and_drives_a_session_through_a_real_executor() {
        let mut host = AgentHost::new();
        let id = host.spawn(SessionId::new("agent-1"), now_host());
        assert!(host.contains(&id));
        assert_eq!(host.len(), 1);

        // Deliver an inbound event — the host drives the whole loop through the real ClockExecutor.
        let outcome = host.deliver(&id, inbound_go(), None);
        assert!(
            matches!(outcome, Some(Ok(()))),
            "a known session runs a turn"
        );

        // The agent ran to completion: it recorded "ran" and left nothing open.
        let hosted = host.get(&id).expect("session registered");
        assert_eq!(hosted.session().kv().get(b"status"), Some(&b"ran"[..]));
        assert_eq!(hosted.open_effects(), 0);
    }

    #[test]
    fn delivering_to_an_unknown_session_is_none_not_a_panic() {
        let mut host = AgentHost::new();
        // No session registered → None (an unknown id is distinct from a loop error).
        assert!(host
            .deliver(&SessionId::new("nope"), inbound_go(), None)
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

    #[test]
    fn two_sessions_run_independently() {
        let mut host = AgentHost::new();
        host.spawn(SessionId::new("a"), now_host());
        host.spawn(SessionId::new("b"), now_host());
        // Drive only "a".
        host.deliver(&SessionId::new("a"), inbound_go(), None);
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
}
