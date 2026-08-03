//! The agent HOST — assembles the kernel building blocks into a process that RUNS agents.
//!
//! This is the milestone the crate exists for: the kernel provides a `Session` (log + KV + the durable
//! dispatch/fold loop), a reducer, a `CompositeExecutor` that routes effects by family string, and an `Authorize`
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
use cdz_kernel::effect::effect_ct;
use cdz_kernel::event::EventBody;
use cdz_kernel::executor::CompositeExecutor;
use cdz_kernel::hash::Hash;
use cdz_kernel::kernel::{KernelError, Session};
use cdz_kernel::reducer::Reducer;
use std::collections::HashMap;

/// A session's identity in the host registry. A short opaque string the operator/driver assigns (e.g.
/// `"concierge"`, `"builder-42"`) — distinct from the kernel's per-effect `EffectId` and from the
/// content `Hash` of the reducer. Owned so the registry key needs no lifetime.
///
/// Backed by `Arc<str>` (operator cheap-clone directive, same as the kernel's `EffectRequest.target`):
/// a `SessionId` is CLONED on every `spawn` (it's the `HashMap` key) and again by `session_ids()`
/// (`keys().cloned()`), so an `Arc<str>` clone is an O(1) refcount bump, not a fresh heap `String`. It
/// derefs to `&str`, so every read/compare is unchanged, and `new` takes `impl Into<Arc<str>>` so
/// `&str`/`String` call sites are unaffected.
//
// The host drives sessions through the kernel's ASYNC loop (`Session::deliver`) so a long fold can
// cooperatively yield and sessions interleave (§15b). A reducer is therefore held as a `Box<dyn
// Reducer>` — the SINGLE reducer trait (operator "one async trait only"): a pure-Rust reducer writes
// a native `impl Reducer` (its `fold` runs to completion with no await point), and a wasm
// reducer uses `AsyncComponentReducer`. Both box directly as `Box<dyn Reducer>` — no wrapper.
#[derive(Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct SessionId(pub std::sync::Arc<str>);

impl SessionId {
    pub fn new(id: impl Into<std::sync::Arc<str>>) -> Self {
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
    /// `reducer` drives folds; `executor` (a by-family-string [`CompositeExecutor`]) performs authorized effects;
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

    /// Attach a §4c mutable-name [`NameStore`](cdz_kernel::name_store::NameStore) so this hosted agent's
    /// `store/set` / `store/resolve` effects work — a builder over [`genesis`](Self::genesis). ADDITIVE:
    /// plain `genesis` leaves the session store-less, so a `store/*` effect there folds an observable `Err`
    /// (never a panic); only an agent that needs the name store calls this.
    ///
    /// v0.2 lifecycle is PER-SESSION: each `HostedSession` owns its own `NameStore` (the kernel seam,
    /// `Session::attach_name_store`, takes it by value — a plain `&mut`-mutated store, not a shared handle).
    /// A shared/federated GLOBAL store (the §4c end-state, "the store is itself a session") is a later
    /// durable-backend slice; it introduces sharing at the persistence layer, not via a host-side lock here.
    ///
    /// The `store/*` effects are still AUTHORIZED by this session's authorizer — grant them with
    /// [`Capability::for_family`](cdz_kernel::effect::Capability::for_family) over
    /// [`STORE_SET`](cdz_kernel::effect::effect_ct::STORE_SET) /
    /// [`STORE_RESOLVE`](cdz_kernel::effect::effect_ct::STORE_RESOLVE) scoped to a name prefix; attaching a
    /// store does NOT grant access.
    pub fn with_name_store(mut self, name_store: cdz_kernel::name_store::NameStore) -> Self {
        self.session.attach_name_store(name_store);
        self
    }

    /// SEED the capability manifest so this agent is "born knowing" its capabilities — call ONCE right
    /// after [`HostedSession::genesis`], before the first [`deliver`](Self::deliver) (host-capability-
    /// discovery I5). The kernel folds a synthetic `control/capabilities` EffectResult (byte-identical to
    /// an on-demand I4b query answer, same code path), so a capability-aware reducer can record its grants
    /// up front without issuing a query. Opt-in: seeding is a separate call, so `genesis` stays sync and an
    /// agent that queries on demand (or doesn't care) needs no change.
    ///
    /// Returns any [`cdz_kernel::effect::ControlEffect`]s the seed turn surfaced; an ordinary reducer
    /// emits none, so most callers ignore the return.
    pub async fn seed_capabilities(&mut self) -> Vec<cdz_kernel::effect::ControlEffect> {
        self.session
            .seed_capabilities(&*self.reducer, &*self.authz, &mut self.executor)
            .await
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
            .deliver(
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
            .fire_due_timers(now_ms, &*self.reducer, &*self.authz, &mut self.executor)
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
    /// report-aware reducer summarizes itself, runs to quiescence, and returns the summary the reducer
    /// emitted as a `control/summary` effect — then DROPS the fork (never persisted). The parent is
    /// provably untouched (the fork is a separate `Session`; this method takes `&self`).
    ///
    /// The summary rides the CONTROL-PLANE return channel (register-by-string beat 3): the reducer emits a
    /// `control/summary` effect (family [`effect_ct::SUMMARY`]) whose `request.payload` carries the summary
    /// bytes; `deliver_control` returns those authz-exempt, non-routed control effects. We scan the
    /// returned `Vec<ControlEffect>` for the `control/summary` entry (FILTERING by family, not taking the
    /// first — `control/capabilities` also rides this channel until it becomes kernel-answered inline) and
    /// read its inline payload. This replaces the earlier `public/summary` KV convention.
    ///
    /// The caller supplies the fork's `reducer` (the same logic the session runs — a `Box<dyn Reducer>`
    /// can't be cloned out of this `HostedSession`, so the caller re-provides it as a `&dyn Reducer`),
    /// a MODEL-ONLY `authz` (a scoped capability so a summarize-fold can call the model but CANNOT take
    /// world-actions — SEC-F1), and an `executor` to serve that model call. Returns `Some(summary_bytes)`
    /// if the reducer emitted a `control/summary` effect with an inline payload, else `None` (it
    /// summarized elsewhere / didn't, emitted a blob payload, or the fork erred).
    pub async fn fork_for_query(
        &self,
        reducer: &dyn Reducer,
        authz: &dyn Authorize,
        executor: &mut CompositeExecutor,
    ) -> Option<Vec<u8>> {
        let mut fork = self.session.fork_for_query();
        // Deliver a `report` inbound so a report-aware reducer (branching on ct.is_report()) summarizes
        // itself. A KernelError here just means no summary (the fork is discarded regardless).
        let body = EventBody::Inbound {
            content_type: cdz_kernel::event::ContentType::report(),
            payload: cdz_kernel::effect::Payload::Inline(Vec::new().into()),
        };
        let controls = fork
            .deliver_control(body, None, reducer, authz, executor)
            .await
            .ok()?;
        // The summary the reducer emitted for observers (§4b tier-1), read off the control-plane channel
        // before the fork drops. Scan for the first `control/summary` effect that ACTUALLY carries inline
        // bytes — folding the inline check into the find (not find-first-by-family THEN check inline), so a
        // leading `control/summary` with a non-inline (blob) payload doesn't mask a later inline one.
        // `control/capabilities` and other control families are skipped by the family match.
        controls
            .into_iter()
            .find_map(|ce| match ce.request.payload {
                Some(cdz_kernel::effect::Payload::Inline(bytes))
                    if ce.request.content_type.matches_family(effect_ct::SUMMARY) =>
                {
                    Some(bytes.to_vec())
                }
                _ => None,
            })
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

/// Assemble the REAL executor set a deployed host runs an agent against (behind `live-net`): the
/// hermetic [`ClockExecutor`] for `Now` plus the two network transports — [`ReqwestHttpTransport`] for
/// `Http` and [`BedrockModelTransport`] for `Model` — wired into a by-family [`CompositeExecutor`]. This
/// is the one-call "give me the live executors" a driver hands to [`HostedSession::genesis`] so a
/// reducer's `Model`/`Http` effects reach the real world — the capstone of the live-net arc: an agent
/// loops against Bedrock + fetches URLs, not stubs.
///
/// Async because the Bedrock transport loads AWS config from the ambient environment (the SDK default
/// provider chain). Credentials + region come from the ENVIRONMENT: environment variables
/// (`AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` / `AWS_SESSION_TOKEN` + region), the shared config/
/// credentials profile, and IMDS — all part of aws-config's DEFAULT chain (not feature-gated). The only
/// credential sources NOT compiled in are SSO and `credentials-process`, which ARE `aws-config`
/// feature-gated (`sso` / `credentials-process`) and we don't enable them. No broker, no credential wiring
/// in code (operator directive: creds from the environment, no Membrain). Returns `Err` if the HTTP client
/// can't be built (e.g. no TLS backend) — a permanent host misconfiguration surfaced at assembly, not
/// per-effect. `Now` stays hermetic (no network); it's included because a real agent reads the clock.
///
/// # Wiring an agent that loops against the real world
///
/// Hand the assembled set to [`HostedSession::genesis`] alongside a reducer + an authorizer; from then on
/// the reducer's `Model` effects reach Bedrock and its `Http` effects reach a real client. This is the
/// end-to-end shape of "an agent runs" (the crate's north star):
///
/// ```no_run
/// # #[cfg(feature = "live-net")]
/// # async fn demo(
/// #     reducer: Box<dyn cdz_kernel::reducer::Reducer>,
/// #     authz: Box<dyn cdz_kernel::authz::Authorize>,
/// #     reducer_hash: cdz_kernel::hash::Hash,
/// #     inbound: cdz_kernel::event::EventBody,
/// # ) -> Result<(), String> {
/// use cdz_agent_host::{live_executor_set, HostedSession};
///
/// // The real executor set: Now (hermetic) + Http (reqwest) + Model (Bedrock, env creds).
/// let executors = live_executor_set().await?;
/// let mut session = HostedSession::genesis(reducer_hash, reducer, authz, executors);
///
/// // Delivering an inbound event runs one full turn: fold → authorize → dispatch (a real Bedrock/HTTP
/// // call) → fold the result back. The agent is running against the world.
/// session
///     .deliver(inbound, None)
///     .await
///     .map_err(|e| format!("turn failed: {e:?}"))?;
/// # Ok(())
/// # }
/// ```
#[cfg(feature = "live-net")]
pub async fn live_executor_set() -> Result<CompositeExecutor, String> {
    use crate::{
        BedrockModelTransport, ClockExecutor, HttpExecutor, ModelExecutor, ReqwestHttpTransport,
    };
    let http = ReqwestHttpTransport::new()?;
    let model = BedrockModelTransport::new().await;
    Ok(CompositeExecutor::new()
        .with_effect(effect_ct::NOW, Box::new(ClockExecutor::new()))
        .with_effect(effect_ct::HTTP, Box::new(HttpExecutor::new(http)))
        .with_effect(effect_ct::MODEL, Box::new(ModelExecutor::new(model))))
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
    /// summarizes itself from that local KV and emits the summary as a `control/summary` effect (the
    /// fork-for-query control-plane pattern, register-by-string beat 3 — no model call, the cheap tier-1
    /// path). The summary bytes ride the effect's payload; the family drives it (kind is irrelevant for a
    /// control family).
    struct ReportingAgent;
    #[async_trait::async_trait(?Send)]
    impl Reducer for ReportingAgent {
        async fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { content_type, .. } if content_type.is_report() => {
                    // Summarize from local KV — here, echo the recorded phase into the emitted summary.
                    let phase = kv.get(b"phase").map(|v| v.to_vec()).unwrap_or_default();
                    let mut summary = b"phase=".to_vec();
                    summary.extend_from_slice(&phase);
                    // A control family drives routing directly — register-by-string, no EffectKind.
                    let request = EffectRequest::new_with_family(
                        effect_ct::SUMMARY,
                        "self",
                        Some(Payload::Inline(summary.into())),
                        Timeliness::Interactive,
                    );
                    FoldOutput::with(vec![request])
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
        let live_event_count = hosted.session().log().len();

        // Fork-for-query it: caller supplies the same (native Reducer) reducer + a model-only authz
        // (deny_all here — the summarize fold takes no world-effects; the control/summary effect is
        // authz-exempt) + an executor. Returns the summary carried on the control-plane channel.
        let mut exec = CompositeExecutor::new();
        let summary = hosted
            .fork_for_query(&ReportingAgent, &Authorizer::deny_all(), &mut exec)
            .await;
        assert_eq!(
            summary.as_deref(),
            Some(&b"phase=working"[..]),
            "the fork summarizes the copied KV state onto the control/summary channel"
        );

        // NON-INTERFERENCE: the live session is byte-for-byte unchanged — the fork's report turn left no
        // trace on it (no new phase write, no summary), and its log didn't grow (the fork is a separate
        // Session; fork_for_query took &self).
        let hosted = host.get(&id).unwrap();
        assert_eq!(hosted.session().kv().get(b"phase"), Some(&b"working"[..]));
        assert_eq!(hosted.session().log().len(), live_event_count);
    }

    /// A report-aware agent that emits control effects on a `report` — but emits `control/capabilities`
    /// FIRST and `control/summary` SECOND, plus a non-summary payload on the capabilities one. Proves the
    /// fork reads the summary by FILTERING on family, not by taking the first control effect.
    struct MultiControlAgent;
    #[async_trait::async_trait(?Send)]
    impl Reducer for MultiControlAgent {
        async fn fold(&self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { content_type, .. } if content_type.is_report() => {
                    let caps = EffectRequest::new_with_family(
                        effect_ct::CAPABILITIES,
                        "self",
                        Some(Payload::Inline(b"NOT-the-summary".to_vec().into())),
                        Timeliness::Interactive,
                    );
                    let summary = EffectRequest::new_with_family(
                        effect_ct::SUMMARY,
                        "self",
                        Some(Payload::Inline(b"the-real-summary".to_vec().into())),
                        Timeliness::Interactive,
                    );
                    // capabilities FIRST, summary SECOND — a take-first read would grab the wrong one.
                    FoldOutput::with(vec![caps, summary])
                }
                _ => FoldOutput::none(),
            }
        }
    }

    #[tokio::test]
    async fn fork_for_query_picks_control_summary_by_family_not_the_first_control_effect() {
        // The reshape FILTERS the returned Vec<ControlEffect> by family == SUMMARY (control/capabilities
        // also rides this channel until it's kernel-answered). Emit capabilities-then-summary so a
        // take-first read would return the capabilities payload; assert we get the summary.
        let mut host = AgentHost::new();
        let id = SessionId::new("multi");
        host.spawn(
            id.clone(),
            HostedSession::genesis(
                Hash::of(b"multi-control-v1"),
                Box::new(MultiControlAgent),
                Box::new(Authorizer::deny_all()),
                CompositeExecutor::new(),
            ),
        );
        let mut exec = CompositeExecutor::new();
        let summary = host
            .get(&id)
            .unwrap()
            .fork_for_query(&MultiControlAgent, &Authorizer::deny_all(), &mut exec)
            .await;
        assert_eq!(
            summary.as_deref(),
            Some(&b"the-real-summary"[..]),
            "must select control/summary by family, not the first (control/capabilities) control effect"
        );
    }

    /// A report-aware agent that never emits a `control/summary` (it does other work on a report but
    /// publishes no summary) — the fork must return `None`, not panic or return some other effect.
    struct NoSummaryAgent;
    #[async_trait::async_trait(?Send)]
    impl Reducer for NoSummaryAgent {
        async fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
            if let EventBody::Inbound { content_type, .. } = &event.body {
                if content_type.is_report() {
                    // Does local work on the report, but emits NO control/summary effect.
                    kv.put(b"noted".to_vec(), b"1".to_vec());
                }
            }
            FoldOutput::none()
        }
    }

    #[tokio::test]
    async fn fork_for_query_returns_none_when_no_control_summary_is_emitted() {
        // The `None` branch: a reducer that summarizes nowhere (emits no control/summary) yields None —
        // the honest "it didn't summarize" signal, replacing the old public/summary-absent path.
        let mut host = AgentHost::new();
        let id = SessionId::new("silent");
        host.spawn(
            id.clone(),
            HostedSession::genesis(
                Hash::of(b"no-summary-v1"),
                Box::new(NoSummaryAgent),
                Box::new(Authorizer::deny_all()),
                CompositeExecutor::new(),
            ),
        );
        let mut exec = CompositeExecutor::new();
        let summary = host
            .get(&id)
            .unwrap()
            .fork_for_query(&NoSummaryAgent, &Authorizer::deny_all(), &mut exec)
            .await;
        assert_eq!(summary, None, "no control/summary emitted → None");
    }

    /// A report-aware agent that emits TWO `control/summary` effects: the first with a BLOB payload (no
    /// inline bytes), the second with the real inline summary. Guards the fix for PR #1641's silent-drop
    /// edge — reading must not stop at the first family match and see a non-inline payload, but scan on to
    /// the inline one.
    struct BlobThenInlineSummaryAgent;
    #[async_trait::async_trait(?Send)]
    impl Reducer for BlobThenInlineSummaryAgent {
        async fn fold(&self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { content_type, .. } if content_type.is_report() => {
                    // First control/summary: a BLOB payload (no inline bytes to read).
                    let blob = EffectRequest::new_with_family(
                        effect_ct::SUMMARY,
                        "self",
                        Some(Payload::Blob(Hash::of(b"summary-blob"))),
                        Timeliness::Interactive,
                    );
                    // Second control/summary: the real inline bytes.
                    let inline = EffectRequest::new_with_family(
                        effect_ct::SUMMARY,
                        "self",
                        Some(Payload::Inline(b"inline-summary".to_vec().into())),
                        Timeliness::Interactive,
                    );
                    FoldOutput::with(vec![blob, inline])
                }
                _ => FoldOutput::none(),
            }
        }
    }

    #[tokio::test]
    async fn fork_for_query_skips_a_blob_summary_and_reads_a_later_inline_one() {
        // PR #1641 fix: the read folds the inline check into the scan (find_map), so a leading
        // control/summary with a non-inline payload does NOT mask a later inline summary. The old
        // find-by-family-then-check-inline returned None here.
        let mut host = AgentHost::new();
        let id = SessionId::new("blob-then-inline");
        host.spawn(
            id.clone(),
            HostedSession::genesis(
                Hash::of(b"blob-then-inline-v1"),
                Box::new(BlobThenInlineSummaryAgent),
                Box::new(Authorizer::deny_all()),
                CompositeExecutor::new(),
            ),
        );
        let mut exec = CompositeExecutor::new();
        let summary = host
            .get(&id)
            .unwrap()
            .fork_for_query(
                &BlobThenInlineSummaryAgent,
                &Authorizer::deny_all(),
                &mut exec,
            )
            .await;
        assert_eq!(
            summary.as_deref(),
            Some(&b"inline-summary"[..]),
            "a leading blob-payload control/summary must not mask a later inline one"
        );
    }

    /// A capability-aware agent: when it sees a capabilities-manifest `EffectResult` (the answer to a
    /// `control/capabilities` query — or the I5 born-knowing seed, same wire shape), it records the raw
    /// manifest bytes into KV under `capabilities`. Lets the seed test assert the guest was born knowing.
    struct CapabilityAwareAgent;
    #[async_trait::async_trait(?Send)]
    impl Reducer for CapabilityAwareAgent {
        async fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
            if let EventBody::EffectResult {
                result: EffectOutcome::Ok(Some(Payload::Inline(bytes))),
                ..
            } = &event.body
            {
                kv.put(b"capabilities".to_vec(), bytes.to_vec());
            }
            FoldOutput::none()
        }
    }

    #[tokio::test]
    async fn seed_capabilities_makes_a_session_born_knowing() {
        // I5 host adoption: seed_capabilities() right after genesis folds a synthetic capabilities-manifest
        // EffectResult (same code path as an on-demand control/capabilities query), so a capability-aware
        // reducer records its grants before the first deliver — without issuing a query.
        //
        // control/capabilities is KERNEL-answered inline: the manifest is computed from the executor's
        // served families ∩ the authorizer's decision and folded back without routing to any executor. So
        // the seed does NOT consult a per-effect grant — deny_all() here PROVES that (a real effect under
        // deny_all would be denied, but the control seed still folds its manifest). The executor serves Now
        // only, so the manifest reflects that mechanism.
        let served =
            || CompositeExecutor::new().with_effect(effect_ct::NOW, Box::new(ClockExecutor::new()));
        let mut hosted = HostedSession::genesis(
            Hash::of(b"cap-aware-v1"),
            Box::new(CapabilityAwareAgent),
            Box::new(Authorizer::deny_all()),
            served(),
        );
        // Precondition: nothing recorded before the seed.
        assert_eq!(hosted.session().kv().get(b"capabilities"), None);

        // Seeding surfaces no ControlEffects (answered inline) — an ordinary caller ignores the return.
        let surfaced = hosted.seed_capabilities().await;
        assert!(
            surfaced.is_empty(),
            "the seed is answered inline, not surfaced"
        );

        // Born knowing: the reducer recorded the seeded payload. Assert it IS the capabilities manifest for
        // this session's mechanism ∩ policy — the exact bytes the kernel projects from the SAME served
        // families + authorizer via its public API — not merely "some non-empty payload".
        let expected = {
            let exec = served();
            let manifest = cdz_kernel::effect::project_manifest(
                cdz_kernel::effect::effect_ct::ALL,
                |f| exec.handles_family(f),
                &Authorizer::deny_all(),
                cdz_kernel::effect::effect_ct::probe_target,
            )
            .await;
            cdz_kernel::event_ast::encode_capability_manifest(&manifest)
        };
        assert_eq!(
            hosted.session().kv().get(b"capabilities"),
            Some(&expected[..]),
            "the seed folds THE capabilities manifest (mechanism ∩ policy) — born knowing, not just any payload"
        );
    }
}
