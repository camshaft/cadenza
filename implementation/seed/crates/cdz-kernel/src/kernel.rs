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
use tracing::{debug, instrument, warn};

/// Errors the kernel surfaces to its driver. Kept small; grows with features.
#[derive(Debug, PartialEq, Eq)]
pub enum KernelError {
    /// An event was appended out of sequence (log corruption / programming error).
    NonContiguousSeq { expected: u64, got: u64 },
    /// The first event of a session must be `Genesis`.
    MissingGenesis,
    /// A fold was attempted on a TERMINATED session (its log tail is
    /// [`EventBody::Terminated`](crate::event::EventBody::Terminated)). §lifecycle I1: a terminated
    /// session refuses every further delivery — a first-class kernel guard so a terminated session can't
    /// be re-driven even by a buggy host. Terminal: there is no recovery (a fresh spawn replaces it, §7).
    FoldRefused,
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
    /// The §4c mutable-name store this session's `store/*` effects act on, if attached via
    /// [`Session::attach_name_store`]. `None` = no store bound, so a `store/set`/`store/resolve` folds an
    /// Err outcome (§9d anti-stuck: an unroutable store effect is an observable failure, never a panic).
    /// A per-session handle for v0 (the shared/federated global store — §4c "the store is itself a
    /// session" — layers behind this same seam later). NOT rebuilt on replay: the store is EXTERNAL
    /// mutable state, not derived from this session's log (unlike kv/armed_timers), so the driver
    /// re-attaches it after `recover`, exactly like `attach_log`.
    name_store: Option<crate::name_store::NameStore>,
    /// The last capability manifest the kernel projected for this session (from the most recent
    /// seed/query/change-push inline answer) — the BASELINE the I6 reactive push diffs against via
    /// [`crate::effect::CapabilityManifest::grant_changes`] to decide whether a capability change is
    /// worth pushing. `None` until the first projection. EPHEMERAL kernel state, NOT replay-rebuilt: the
    /// manifest bytes ARE in the log (the answer EffectResult), but the kernel never re-parses its own
    /// control payloads (see [`crate::event_ast::encode_capability_manifest`]), so this cache is repopulated
    /// by live projection activity, not by replay. A freshly-recovered session has `None` until its next
    /// projection — safe, because the first push after recovery simply has no baseline to suppress against
    /// (it pushes iff there's anything to know), and steady-state pushes diff against the live baseline.
    last_manifest: Option<crate::effect::CapabilityManifest>,
}

impl Session {
    /// Start a fresh ROOT session with a genesis event naming the reducer + a caller-supplied per-spawn
    /// `spawn_nonce` (entropy). The genesis is the first log entry; nothing is folded yet.
    ///
    /// `spawn_nonce` is what makes [`genesis_hash`](Self::genesis_hash) per-SESSION unique (§lifecycle I2 /
    /// operator ruling): the kernel is clock-free + entropy-free (§9c), so the HOST mints the nonce and
    /// passes it. `spawn_nonce` is a `Hash`, so the host DERIVES it as `Hash::of(<spawn-unique bytes>)`
    /// (e.g. `Hash::of(&getrandom_bytes)` — OS entropy, not wall-clock), NOT `Hash::from_bytes(random)`.
    /// It's recorded in the durable seq-0 event, so recovery/[`replay`] reads it
    /// FROM the log (never re-mints — that would change the id on recovery). A ROOT session has no parent;
    /// use [`genesis_spawned`](Self::genesis_spawned) for a session spawned by another (`lifecycle/spawn`).
    pub fn genesis(reducer: Hash, spawn_nonce: Hash) -> Self {
        Self::genesis_spawned(reducer, spawn_nonce, None)
    }

    /// Start a fresh session with explicit `parent` provenance — the SPAWNED-child constructor (§6/§I2).
    /// `parent` = `Some(<parent session's genesis hash>)` for a child spawned via `lifecycle/spawn`, so the
    /// child's own genesis-hash is provenance-dependent (self-certifies its parent); `None` = a root
    /// session (equivalent to [`genesis`](Self::genesis)). The durable `Spawned{child_hash}` edge in the
    /// PARENT's log is the other half of the relation.
    pub fn genesis_spawned(reducer: Hash, spawn_nonce: Hash, parent: Option<Hash>) -> Self {
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
            name_store: None,
            last_manifest: None,
        };
        // The SAME genesis-event construction `derive_genesis_hash` hashes — so a host pre-computing the
        // child's SessionId and this kernel registering it can never disagree (single source of truth).
        s.log
            .push(Self::genesis_event(reducer, spawn_nonce, parent));
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

    /// Attach the §4c mutable-name [`crate::name_store::NameStore`] this session's `store/*` effects act on
    /// (the drive loop routes `store/set`/`store/resolve` to it — see `drive_worklist`). Like
    /// [`Session::attach_log`], the store is EXTERNAL state re-attached by the driver after `recover` (it's
    /// not rebuilt from the log). Without it, a `store/*` effect folds an observable Err (§9d), never panics.
    pub fn attach_name_store(&mut self, name_store: crate::name_store::NameStore) {
        self.name_store = Some(name_store);
    }

    /// Borrow this session's attached §4c [`NameStore`](crate::name_store::NameStore) (`None` if none is
    /// attached). The READ-BACK dual of [`Session::attach_name_store`]: `attach` hands a store IN by value,
    /// this hands a `&` back OUT so a driver can observe what the session's `store/set`s mutated WITHOUT
    /// taking the store away (the session keeps driving). This is the seam the host's shared-store slice
    /// needs — after session A `store/set COMPILER_LATEST → hash`, the host reads A's store here, exports it
    /// with [`crate::name_store::NameStore::to_set_entries`], and seeds session B via
    /// [`crate::name_store::NameStore::replay_set_entries`], so B can `store/resolve` the pointer A published
    /// (the "agent B runs the program the resolver fetched" loop). Borrowing, so it composes with the
    /// by-value attach without a shared handle / interior mutability — the driver owns the sharing policy.
    pub fn name_store(&self) -> Option<&crate::name_store::NameStore> {
        self.name_store.as_ref()
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
    /// [`Session::deliver`]es a report/summarize message, runs to quiescence, reads the reducer's answer
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
        // REUSE the parent's genesis provenance (spawn_nonce + parent), NOT a fresh spawn: a query-fork is
        // an ephemeral read-view of THIS session, never a registered spawn, so it must carry the parent's
        // identity — minting a new nonce would give it a distinct genesis_hash / SessionId, which is wrong
        // for a read-view (§lifecycle I2, v-agent-harness-host confirm). It's never registered, so there's
        // no id collision from sharing the parent's genesis identity.
        let (spawn_nonce, parent) = self.genesis_provenance();
        let mut fork = Session::genesis_spawned(self.reducer_hash(), spawn_nonce, parent);
        fork.kv = self.kv.clone();
        fork.last_now = self.last_now;
        fork
    }

    /// The genesis event's `(spawn_nonce, parent)` provenance (§lifecycle I2). Panics on a non-Genesis head
    /// — the same internal invariant [`reducer_hash`](Self::reducer_hash)/[`genesis_hash`](Self::genesis_hash)
    /// guard (a `Session` only exists via a genesis constructor / `replay`, both of which guarantee it).
    fn genesis_provenance(&self) -> (Hash, Option<Hash>) {
        match self.log.first().map(|e| &e.body) {
            Some(EventBody::Genesis {
                spawn_nonce,
                parent,
                ..
            }) => (*spawn_nonce, *parent),
            _ => panic!(
                "cdz-kernel invariant violated: session log's first event is not Genesis \
                 (a Session is only constructed via a genesis constructor/replay, which guarantee it)"
            ),
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
            Some(EventBody::Genesis { reducer, .. }) => *reducer,
            _ => panic!(
                "cdz-kernel invariant violated: session log's first event is not Genesis \
                 (a Session is only constructed via genesis()/replay(), both of which guarantee it)"
            ),
        }
    }

    /// The hash of this session's GENESIS event (`log[0]`), via [`Event::hash`]. Distinct from
    /// [`reducer_hash`](Self::reducer_hash), which reads only the reducer *inside* genesis — this hashes
    /// the whole genesis event and is STABLE across replay (genesis is the log's frozen head).
    ///
    /// Intended as the host's `SessionId` (operator ruling 2026-08-06: "session ids = a hash … as long as
    /// the genesis is unique" → `SessionId = genesis-hash`), which collapses name-addressing to an identity
    /// map (§4c `name → hash` IS `name → SessionId`, no host-side lookup).
    ///
    /// PER-SESSION UNIQUE (§lifecycle I2 — the operator's "as long as genesis is unique" condition is now
    /// MET): the genesis event carries a caller-supplied `spawn_nonce` (host-minted getrandom entropy) plus
    /// optional `parent` provenance, so this hash is a function of `(reducer, spawn_nonce, parent)`, not the
    /// reducer alone. Two sessions over the SAME reducer get DIFFERENT ids as long as the host mints a fresh
    /// nonce per spawn (which it must — see [`genesis`](Self::genesis)); a spawned child additionally differs
    /// by its parent hash. So `SessionId = genesis-hash` is a sound identity, and name-addressing (§4c
    /// `name → hash`) IS `name → SessionId` with no host-side lookup.
    ///
    /// Panics on a log whose first event isn't Genesis — the same internal invariant `reducer_hash`
    /// guards (a `Session` only exists via `genesis()`/`replay()`, both of which guarantee it).
    pub fn genesis_hash(&self) -> Hash {
        match self.log.first() {
            Some(
                e @ Event {
                    body: EventBody::Genesis { .. },
                    ..
                },
            ) => e.hash(),
            _ => panic!(
                "cdz-kernel invariant violated: session log's first event is not Genesis \
                 (a Session is only constructed via genesis()/replay(), both of which guarantee it)"
            ),
        }
    }

    /// This session's PARENT — the spawning session's [`genesis_hash`](Self::genesis_hash) (= its SessionId),
    /// or `None` for a ROOT session (spawned by no one). Read from the Genesis event's parent-provenance
    /// (§lifecycle I2); the pub read-accessor dual of the private `genesis_provenance`, mirroring
    /// [`genesis_hash`](Self::genesis_hash)/[`name_store`](Self::name_store). Read-only, no behavior change.
    ///
    /// The HOST uses this to route a terminated child's terminal-outcome signal (`ChildExited`, §lifecycle
    /// I7 "parent observes a child's terminal outcome") back to its parent's inbox: on terminate, look up the
    /// dead child's `parent()` and, if `Some`, emit a supervision Inbound to it (the parent's userspace
    /// supervisor reducer then folds it — restart/escalate). `None` = a root with no parent to notify.
    pub fn parent(&self) -> Option<Hash> {
        self.genesis_provenance().1
    }

    /// The genesis event a fresh session over `(reducer, spawn_nonce, parent)` WOULD carry as `log[0]` —
    /// the single construction site both [`genesis_spawned`](Self::genesis_spawned) (which pushes it) and
    /// [`derive_genesis_hash`](Self::derive_genesis_hash) (which hashes it) route through, so the two can
    /// NEVER diverge on field order / seq / cause.
    fn genesis_event(reducer: Hash, spawn_nonce: Hash, parent: Option<Hash>) -> Event {
        Event {
            seq: 0,
            cause: None,
            body: EventBody::Genesis {
                reducer,
                spawn_nonce,
                parent,
            },
        }
    }

    /// Compute the genesis-hash (= [`SessionId`](Self::genesis_hash)) a session over
    /// `(reducer, spawn_nonce, parent)` WILL have — WITHOUT constructing the `Session`. This is the
    /// host-reproducible derivation seam (§lifecycle I3, v-agent-harness-host coordination 2026-08-06): the
    /// `lifecycle/spawn` executor PRE-COMPUTES the child's provisional SessionId to return synchronously
    /// (option-b: spawn defers the registry mutation to the loop, but the sync `Ok(payload=child_hash)`
    /// contract + `SessionId = genesis-hash-hex` must hold), then the loop instantiates the child via
    /// [`genesis_spawned`](Self::genesis_spawned) with the SAME `(reducer, spawn_nonce, parent)`. Because
    /// BOTH paths hash the SAME [`genesis_event`](Self::genesis_event), the pre-computed provisional id is
    /// GUARANTEED byte-identical to what [`genesis_hash`](Self::genesis_hash) reports on the registered
    /// child — the host never reimplements the derivation (field order / encoding / hash algo live in ONE
    /// place here, so a future Genesis-shape change can't silently drift the two apart).
    pub fn derive_genesis_hash(reducer: Hash, spawn_nonce: Hash, parent: Option<Hash>) -> Hash {
        Self::genesis_event(reducer, spawn_nonce, parent).hash()
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
                        target: target.to_string(),
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
    /// awaiting the reducer's fold via [`Reducer`]. Appends
    /// `body` (cause-linked to `cause`), then folds it through `reducer`; each effect the fold emits is
    /// authorized then handled by kind: an executor-dispatched effect (Http/Model/Shell/Now/Emit) is
    /// durably dispatched, performed by the executor, and its result folded back; a `Timer` effect is
    /// ARMED (a durable `TimerArmed`, no executor call) and fired later by [`Session::fire_due_timers`].
    /// The loop runs until no new effects remain. Because the fold is `.await`ed, a long-running wasm fold
    /// cooperatively YIELDS at fuel intervals (see [`crate::wasm_host::AsyncComponentReducer`]) rather than
    /// blocking the caller's single-threaded loop.
    ///
    /// The executor is invoked synchronously (`&mut dyn Executor`): only the reducer fold awaits. The
    /// guest-facing WIT ABI is blocking — the async lives entirely in the host-side Rust driving the
    /// component, never in the guest's interface.
    pub async fn deliver(
        &mut self,
        body: EventBody,
        cause: Option<Hash>,
        reducer: &dyn Reducer,
        authz: &(impl Authorize + ?Sized),
        executor: &mut (impl Executor + ?Sized),
    ) -> Result<(), KernelError> {
        // The common delivery path DROPS the surfaced control/* effects (a live session turn doesn't
        // consume them by default). A driver that needs them — `fork_for_query`'s summary watch — calls
        // [`Session::deliver_control`] instead. Keeping this `-> Result<(), _>` is the never-red
        // bridge: the downstream `cdz-agent-host` HostedSession::deliver returns this verbatim, so widening
        // it would break its build; the control-returning variant is ADDITIVE alongside it.
        self.deliver_control(body, cause, reducer, authz, executor)
            .await
            .map(|_control| ())
    }

    /// Like [`Session::deliver`], but RETURNS the `control/*` effects the reducer emitted this turn
    /// (control-plane partition, register-by-string): `control/*` families are authz-exempt + not routed —
    /// the kernel collects them and hands them back here for the DRIVER to consume (e.g. `fork_for_query`
    /// scrapes the `control/summary` effect's `request.payload`). The common [`Session::deliver`] path drops
    /// them; use this when you need them. See [`crate::effect::ControlEffect`].
    // Observability (facade only — the kernel never installs a subscriber; v-ah-host owns that). The span
    // covers a full delivery turn. `skip_all` because the args are trait objects / an opaque event payload
    // (not `Debug`, and the payload is guest bytes we must not log wholesale — §PII/size); instead record
    // the event-body VARIANT NAME + the event count, both non-sensitive. A `now`/`http` payload's contents
    // never enter a span field. With no subscriber registered every span/event is a near-free atomic load.
    #[instrument(
        level = "debug",
        name = "kernel.deliver",
        skip_all,
        fields(event = event_body_name(&body), log_len = self.log.len())
    )]
    pub async fn deliver_control(
        &mut self,
        body: EventBody,
        cause: Option<Hash>,
        reducer: &dyn Reducer,
        authz: &(impl Authorize + ?Sized),
        executor: &mut (impl Executor + ?Sized),
    ) -> Result<Vec<crate::effect::ControlEffect>, KernelError> {
        // §lifecycle I1: a TERMINATED session refuses every further fold — checked BEFORE the append so a
        // terminated log stays frozen (no event is written, no reducer runs). First-class kernel guard, not
        // a host convention: even a buggy/hostile driver can't re-drive a terminated session. Terminal +
        // replay-stable (the tail is durable, so a recovered session whose tail is `Terminated` refuses too).
        if self.is_terminated() {
            return Err(KernelError::FoldRefused);
        }
        self.append(body, cause).await;
        let control = self.drive(reducer, authz, executor).await;
        Ok(control)
    }

    /// Is this session TERMINATED — i.e. its log TAIL is an [`EventBody::Terminated`] marker (§lifecycle
    /// I1)? Keys on the LAST event, not "any Terminated anywhere": terminality is a terminal state the
    /// marker installs as the tail, and nothing can be appended after it (the fold guard rejects), so the
    /// tail is the authoritative signal — and it's replay-stable (the durable log rebuilds the same tail).
    pub fn is_terminated(&self) -> bool {
        matches!(
            self.log.last().map(|e| &e.body),
            Some(EventBody::Terminated { .. })
        )
    }

    /// TERMINATE this session (§lifecycle I1): append the durable [`EventBody::Terminated`] marker as the
    /// log tail WITHOUT folding it through the reducer — the fold-free public seam the host's
    /// `lifecycle/terminate` executor drives (a terminal marker must be INSTALLED as the frozen tail, not
    /// folded like an inbound). This is the public counterpart to the crate-private `append` the I1 tests
    /// use; `cdz-agent-host` calls THIS (it can't reach `append`, and `deliver` would fold the marker).
    ///
    /// `by` is the terminating controller's session identity (its genesis hash = its SessionId); `reason`
    /// is a diagnostic string. The marker is cause-linked to the current tip (the causal-DAG edge, §5) and
    /// persisted through the attached log store like any other append. Returns the marker's event hash so
    /// the caller can cause-link / log it.
    ///
    /// IDEMPOTENT-BY-REJECTION: terminating an ALREADY-terminated session is a no-op that returns
    /// [`KernelError::FoldRefused`] — never a second `Terminated` marker (which would break the "the tail
    /// IS the terminal marker" invariant and the durable-once contract). Terminal: there is no un-terminate.
    pub async fn terminate(&mut self, by: Hash, reason: String) -> Result<Hash, KernelError> {
        if self.is_terminated() {
            return Err(KernelError::FoldRefused);
        }
        let cause = Some(self.tip_hash());
        Ok(self
            .append(EventBody::Terminated { by, reason }, cause)
            .await)
    }

    /// RECORD a parent→child spawn edge (§lifecycle I2 / §6): append an [`EventBody::Spawned`] naming the
    /// child by its genesis hash into THIS (the parent's) log — the fold-free public seam the host's
    /// `lifecycle/spawn` executor drives AFTER it instantiates the child (so the durable tree edge is on
    /// the parent's log). Like [`terminate`](Self::terminate) it appends without folding (a recorded fact,
    /// not a reducer input) + persists through the store; cause-linked to the current tip. Returns the
    /// edge event's hash. REFUSED on a terminated parent ([`KernelError::FoldRefused`]) — a dead session
    /// can't spawn.
    pub async fn record_spawn(&mut self, child_hash: Hash) -> Result<Hash, KernelError> {
        if self.is_terminated() {
            return Err(KernelError::FoldRefused);
        }
        let cause = Some(self.tip_hash());
        Ok(self.append(EventBody::Spawned { child_hash }, cause).await)
    }

    /// This session's DIRECT children — the child genesis hashes from its [`EventBody::Spawned`] edges,
    /// in spawn order (§lifecycle I2 / §6). The supervision authority (I6 `DescendantOf`) + cascade (§8)
    /// build on this: a controller's TRANSITIVE descendants are the closure of this over the child
    /// sessions' own logs (each session records only its OWN direct spawns; the tree is assembled by the
    /// host walking session logs). Reads the durable log, so it's replay-stable.
    pub fn spawned_children(&self) -> Vec<Hash> {
        self.log
            .iter()
            .filter_map(|e| match &e.body {
                EventBody::Spawned { child_hash } => Some(*child_hash),
                _ => None,
            })
            .collect()
    }

    /// Genesis-seed the capability manifest (host-capability-discovery I5): fold a synthetic
    /// `control/capabilities` answer so the guest knows its capabilities without issuing a query. Synthesizes
    /// a `control/capabilities` [`Effect`] and runs it through the same drive path as a guest-issued query
    /// ([`Session::drive_worklist`]'s inline-answer arm), so there is one manifest shape, one guest decoder, and
    /// one replay-safe (logged EffectResult) code path. Call immediately after [`Session::genesis`], before
    /// the first delivery; the seed's dispatch is cause-linked to the genesis event.
    ///
    /// IDEMPOTENT: a second (or later) call is a NO-OP returning an empty Vec — the "seed once" contract is
    /// ENFORCED, not just documented. Without this, a re-call would append a duplicate manifest dispatch/
    /// result AND cause-link it to the current tip rather than genesis, corrupting the causal provenance.
    ///
    /// Returns any `ControlEffect`s the fold surfaced (a genesis reducer that reacts to the seed by emitting
    /// a control/* effect); an ordinary genesis reducer emits none, so callers can usually ignore it.
    pub async fn seed_capabilities(
        &mut self,
        reducer: &dyn Reducer,
        authz: &(impl Authorize + ?Sized),
        executor: &mut (impl Executor + ?Sized),
    ) -> Vec<crate::effect::ControlEffect> {
        // Enforce seed-once: if the log already carries a control/capabilities dispatch (from a prior seed),
        // this is a no-op — never a duplicate seed or a mis-cause-linked (tip- rather than genesis-anchored)
        // second manifest.
        if self.already_seeded_capabilities() {
            return Vec::new();
        }
        // Synthesize the control/capabilities request the guest would have sent, via the register-by-string
        // constructor (effect-schema slice 2): the family drives everything, and it takes the Emit
        // placeholder kind internally (a control family has no distinguishing EffectKind). The durable
        // Dispatched frame records the CONTROL family (the recovery-classification fix), so the seed is
        // replay-classified as control/capabilities, not a real emit. No continuation token: the seed is
        // kernel-originated, not a reducer continuation.
        let request = EffectRequest::new_with_family(
            crate::effect::effect_ct::CAPABILITIES,
            "self",
            None,
            crate::effect::Timeliness::Interactive,
        );
        // Cause-link the seed to the GENESIS event (session birth) — not the current tip. Conceptually the
        // kernel asks control/capabilities on the guest's behalf at t=0, so genesis is the true cause; and
        // it gives the seed a stable, distinguishing signature (cause==genesis) that a guest-issued query
        // (cause-linked to an Inbound fold) never has — see `already_seeded_capabilities`. In the normal
        // "seed immediately after genesis" call the tip IS genesis, so this only matters if a caller seeds
        // later, but keying on genesis explicitly makes the identity correct regardless of call ordering.
        let cause = self
            .log
            .first()
            .map(|e| e.hash())
            .expect("log always has genesis");
        let seed = Effect {
            request,
            token: None,
        };
        self.drive_worklist(vec![(seed, cause)], reducer, authz, executor)
            .await
    }

    /// Has this session already been capability-SEEDED (by [`seed_capabilities`])? True iff the log
    /// carries a `control/capabilities` `Dispatched` whose cause is the GENESIS event — the seed's specific
    /// signature. This deliberately does NOT match a GUEST-issued `control/capabilities` query, which
    /// appends the same `Dispatched{family}` frame but is cause-linked to an `Inbound` fold, not genesis. A
    /// guest query answers the manifest but is not "the seed", so keying on `family` alone would (a) let a
    /// pre-seed guest query suppress the real seed, and (b) misstate the invariant. Cause-linked to genesis
    /// is the seed's true identity and is replay-stable (the durable cause edge survives recovery).
    fn already_seeded_capabilities(&self) -> bool {
        // A fresh/empty log has no genesis to cause-link against (can't be seeded yet). Guard the empty
        // case here rather than calling `genesis_hash()` (which panics on a non-Genesis head) — a
        // never-constructed session shouldn't panic this read.
        if self.log.is_empty() {
            return false;
        }
        let genesis_hash = self.genesis_hash();
        self.log.iter().any(|e| {
            matches!(
                &e.body,
                EventBody::Dispatched { family, .. }
                    if family.as_ref() == crate::effect::effect_ct::CAPABILITIES
            ) && e.cause == Some(genesis_hash)
        })
    }

    /// Push a `capabilities-changed` to the guest IFF this session's projected capability manifest actually
    /// moved (host-capability-discovery I6b, the kernel half). The DRIVER (host) calls this AFTER mutating the
    /// session's capability surface — an executor added/removed, or the authorizer swapped (e.g. a §4c
    /// `store/set` to the policy pointer) — passing the NEW `authz`/`executor`. The kernel:
    ///
    /// 1. Projects the manifest over [`effect_ct::ALL`](crate::effect::effect_ct::ALL) against the CURRENT
    ///    (post-mutation) `authz` + `executor` — the same projection the seed/query inline arm uses.
    /// 2. Diffs it against the last manifest the guest saw (`last_manifest`) via
    ///    [`CapabilityManifest::grant_changes`](crate::effect::CapabilityManifest::grant_changes).
    /// 3. If the delta is EMPTY → NO-OP, appends nothing, returns an empty Vec. This is the design's "delivered
    ///    ONLY to sessions whose projected manifest actually changed" gate — and it makes coalescing FREE: after
    ///    a burst of N surface mutations, one call here appends at most one push carrying the net manifest (call
    ///    it once per settle point, not per mutation).
    /// 4. If non-empty → folds the NEW manifest back through the SAME inline `control/capabilities` path as
    ///    seed/query (one manifest shape, one guest decoder, replay-safe logged answer), which also refreshes
    ///    `last_manifest`. The push's dispatch is cause-linked to the current tip (a mid-session event, unlike
    ///    the genesis-anchored seed).
    ///
    /// Returns any `ControlEffect`s the fold surfaced (usually none). Idempotent in the sense that matters: a
    /// second call with an unchanged surface is the empty-delta no-op.
    pub async fn push_capabilities_changed(
        &mut self,
        reducer: &dyn Reducer,
        authz: &(impl Authorize + ?Sized),
        executor: &mut (impl Executor + ?Sized),
    ) -> Vec<crate::effect::ControlEffect> {
        // Project the manifest against the CURRENT surface, and gate on whether it moved vs the last one the
        // guest saw. A None baseline (never projected — e.g. a session that was never seeded, or freshly
        // recovered) means "no baseline to suppress against" → the gate below is `!entries.is_empty()`, so a
        // non-empty manifest pushes the initial state. Projecting over `effect_ct::ALL` always yields one entry
        // per family (the entries may all be `Absent`, but the Vec is non-empty), so in practice a first push
        // always fires; the empty guard is the honest, minimal condition (an empty family set → nothing to say).
        let projected = crate::effect::project_manifest(
            crate::effect::effect_ct::ALL,
            |f| executor.handles_family(f),
            authz,
            crate::effect::effect_ct::probe_target,
        )
        .await;
        let changed = match &self.last_manifest {
            Some(prev) => !projected.grant_changes(prev).is_empty(),
            None => !projected.entries.is_empty(),
        };
        if !changed {
            return Vec::new();
        }
        // Something moved — answer a fresh control/capabilities inline (re-projects the SAME manifest, appends
        // the durable Dispatched+EffectResult, folds it to the guest, and refreshes `last_manifest`). Reusing
        // the inline arm keeps ONE manifest shape / decoder / replay path for seed + query + push.
        let request = EffectRequest::new_with_family(
            crate::effect::effect_ct::CAPABILITIES,
            "self",
            None,
            crate::effect::Timeliness::Interactive,
        );
        // A mid-session push is cause-linked to the current tip (the mutation that prompted it), NOT genesis —
        // so it's distinct from the seed (`already_seeded_capabilities` keys the seed on cause==genesis).
        let cause = self
            .log
            .last()
            .map(|e| e.hash())
            .expect("log always has at least genesis");
        let push = Effect {
            request,
            token: None,
        };
        self.drive_worklist(vec![(push, cause)], reducer, authz, executor)
            .await
    }

    /// Fire every armed timer past `now_ms`, driving the [`Reducer`] for each. Determinism (§9c): the FIRED
    /// time is the timer's own deadline, not `now_ms`, so replay reconstructs identically.
    pub async fn fire_due_timers(
        &mut self,
        now_ms: u64,
        reducer: &dyn Reducer,
        authz: &(impl Authorize + ?Sized),
        executor: &mut (impl Executor + ?Sized),
    ) -> usize {
        // §lifecycle I1: a TERMINATED session fires no timers — the terminal marker must stay the log tail,
        // so no `TimerFired` may be appended after it (that would un-tail the marker and flip is_terminated()
        // back to false, breaking the invariant). Return 0 (nothing fired), mirroring the deliver_control
        // refusal. See the guard-every-append-path invariant (github-liaison #2381 review).
        if self.is_terminated() {
            return 0;
        }
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
            )
            .await;
            self.drive(reducer, authz, executor).await;
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
    async fn append(&mut self, body: EventBody, cause: Option<Hash>) -> Hash {
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
                if let Err(e) = store.append(&event).await {
                    self.persist_error = Some(e);
                }
            }
        }
        self.log.push(event);
        hash
    }

    /// Drive one fold→authorize→dispatch→fold-result turn: fold the tip, then work the resulting effects to
    /// quiescence. Reducer folds + the executor call `.await` (so a long wasm fold cooperatively yields).
    async fn drive(
        &mut self,
        reducer: &dyn Reducer,
        authz: &(impl Authorize + ?Sized),
        executor: &mut (impl Executor + ?Sized),
    ) -> Vec<crate::effect::ControlEffect> {
        let trigger = self.tip_hash();
        let initial = self.fold_tip(reducer, trigger).await;
        self.drive_worklist(initial, reducer, authz, executor).await
    }

    /// Work a queue of pending effects to quiescence: for each, apply the control-plane partition, then
    /// authorize → durable dispatch → execute → fold-result (§16c-S1), pushing any effects the fold emits
    /// back onto the queue. Reducer folds (`fold_tip`/`record_result`) + the executor call `.await`.
    async fn drive_worklist(
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
                    // record_result the continuation token to resume the guest. authz-exempt: no
                    // authorize() call; the projection PROBES authz per family but the query itself is free.
                    let idempotency_key = idempotency_key_for(id, &req);
                    let dispatch_hash = self
                        .append(
                            EventBody::Dispatched {
                                id,
                                kind: req.kind.clone(),
                                family: req.content_type.family.as_ref().into(),
                                target: req.target.clone(),
                                idempotency_key,
                                deadline_ms: None,
                                token,
                            },
                            Some(cause),
                        )
                        .await;
                    // S1 latch: an un-durable dispatch folds an Err, never a phantom answer (same tier-B
                    // rule the routed path applies before executing).
                    let outcome = if self.persist_error.is_some() {
                        EffectOutcome::err(
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
                        // Cache the just-projected manifest as the I6 baseline: every inline answer
                        // (seed/query/push) refreshes it, so `push_capabilities_changed` always diffs against
                        // the last manifest the guest actually saw. (Ephemeral — see the `last_manifest` field.)
                        self.last_manifest = Some(manifest);
                        EffectOutcome::Ok(Some(crate::effect::Payload::Inline(bytes.into())))
                    };
                    let more = self
                        .record_result(id, outcome, reducer, dispatch_hash)
                        .await;
                    for pair in more {
                        to_process.push(pair);
                    }
                    continue;
                }
                // FOLD-BACK control (control/signature): the answer must RESUME the emitting reducer's
                // continuation, so give it a Dispatched frame — exactly like a routed effect or the
                // capabilities path above — BEFORE surfacing it. This enters `open` keyed by `id` and records
                // the continuation token, so the host can later settle it by `id` via `settle_control_result`
                // (→ an EffectResult keyed to this token → fold_tip). Without the frame `record_result` would
                // panic (no token to derive) and the effect could never fold back. authz-exempt (control),
                // and NOT answered inline — the host produces the answer (e.g. wasmtime reflection). A
                // fire-and-forget control family (summary) skips this and surfaces with NO frame (below), so
                // it never enters `open` (it would otherwise hang as a never-settled effect).
                if crate::effect::effect_ct::is_fold_back_control(&req.content_type.family) {
                    let idempotency_key = idempotency_key_for(id, &req);
                    self.append(
                        EventBody::Dispatched {
                            id,
                            kind: req.kind.clone(),
                            family: req.content_type.family.as_ref().into(),
                            target: req.target.clone(),
                            idempotency_key,
                            deadline_ms: None,
                            token: token.clone(),
                        },
                        Some(cause),
                    )
                    .await;
                }
                control_out.push(crate::effect::ControlEffect {
                    request: req,
                    token,
                    id,
                });
                continue;
            }

            // SEC-F1: authorize against the resolved target, awaiting the (possibly wasm) policy gate.
            // For a `store/*` effect the "target" is the mutable NAME, so this gate IS the §4c write-
            // authority check: a capability whose family is `store/*` and whose predicate admits the name
            // (e.g. `Prefix("system/")`) permits `store/set system/…`; a session without it is denied here,
            // exactly as an unauthorized HTTP host would be. So store effects reuse the ONE authz seam.
            //
            // SHELL PIPELINE FAN-OUT (operator security directive): a `shell` effect carrying a structured
            // `(shell-pipeline …)` payload is authorized STAGE-BY-STAGE — each stage's PROGRAM is the resolved
            // target of one authz call, deny-all-if-ANY-denied — instead of the single bare `target`. This
            // keeps the ONE SEC-F1 authz seam AND an UNCHANGED [`Authorize`] signature: the authorizer still
            // gates one program-target per call, so the v0 predicate authorizer and a future Cedar-as-wasm
            // authorizer both gate a whole pipeline with ZERO pipeline-codec knowledge (a stage program is
            // just another target). A bare-target shell (no payload) + every other effect take the single-
            // target authorize. Same "the target is what authz gated" discipline as the store name check
            // below (a stage whose program the authorizer denies can never be dispatched).
            let authz_result = match authorize_shell_pipeline(&req, authz).await {
                // The payload decoded as a `(shell-pipeline …)` → the per-stage fan-out verdict IS the gate:
                // each stage's PROGRAM was authorized (the SEC-F1 unit), deny-all-if-any-denied. `req.target`
                // is NOT gated for a pipeline — it is vestigial on the pipeline path.
                //
                // TARGET-GATE RELAX (co-landed with the host pipeline executor). Earlier (reviewer HIGH on
                // #2596) the kernel ALSO gated `req.target` here — a belt-and-suspenders check — because no
                // pipeline-executing host consumer had landed and the host `ShellExecutor` still direct-exec'd
                // `req.target`; without the extra gate a reducer could pair a DENIED `target` with an ALLOWED
                // one-stage pipeline payload, the stages would authorize, and the host would run the ungated
                // denied `target`. The host pipeline executor has now landed (`cdz-agent-host` shell.rs): it
                // keys on the SAME "payload decodes as (shell-pipeline …)" discriminant and runs the decoded
                // STAGES, never `req.target`, on the pipeline path. So the belt-and-suspenders `target` gate is
                // no longer load-bearing — `req.target` is unreachable by the host for a pipeline — and gating
                // it would only reject pipelines whose (unused) `target` a policy happens to deny, a spurious
                // denial. The per-stage fan-out remains the sole, complete SEC-F1 gate for the pipeline path.
                Some(stage_verdict) => stage_verdict,
                // NOT a pipeline (a bare-target shell, an opaque non-pipeline payload like an M3 tool-call's
                // raw input, a blob-ref, or any non-shell effect) → the ordinary single-target SEC-F1 gate on
                // `req.target`. Backward-compatible: today's single-command shell + every other effect are
                // unchanged. The host MUST use the SAME "decodes as (shell-pipeline …)" discriminant so a guest
                // can't get multi-stage execution without a decodable pipeline (which is what triggers fan-out).
                None => authz.authorize(&req).await,
            };
            if let Err(reason) = authz_result {
                // SEC-F1 denial — WARN (an authorization boundary was hit). Log only NON-SENSITIVE
                // metadata: `target` is GUEST-controlled (a URL with a token/query, a PII path) and
                // `reason` is formatted FROM it (authz.rs), so neither goes into the tracing stream —
                // tracing ships off-box (v-ah-host owns the subscriber/export), and a guest secret must
                // not leak into telemetry (#2169/#2180 MED, untrusted-input-in-output class as #2050/#2090).
                // `family` is ALSO guest-controllable for an extension family (a `Cow::Owned` register-by-
                // string name), so it goes through `loggable_family` (well-known static name verbatim,
                // extension → "<extension>" + its length) — #2180 residual. The full target + reason are in
                // the DURABLE `EventBody::AuthzDenied` below (on-box audit), which never leaves the box.
                warn!(
                    effect_id = id.0,
                    family = loggable_family(&req.content_type.family),
                    family_len = req.content_type.family.len(),
                    target_len = req.target.len(),
                    "effect authorization denied"
                );
                let denial_hash = self
                    .append(EventBody::AuthzDenied { id, reason, token }, Some(cause))
                    .await;
                for pair in self.fold_tip(reducer, denial_hash).await {
                    to_process.push(pair);
                }
                continue;
            }

            // §4c STORE PARTITION (slice 3b): a `store/*` family is not executor-routed — the kernel
            // applies it to the attached mutable-name store (like a control family is kernel-answered, but
            // store IS authz-gated, so it lands AFTER the authorize() above). Durable Dispatched BEFORE the
            // mutation (S1), then apply_effect, then fold the outcome. No store attached, or a malformed
            // effect, is an observable Err (§9d anti-stuck), never a panic.
            if crate::effect::effect_ct::is_store_family(&req.content_type.family) {
                let idempotency_key = idempotency_key_for(id, &req);
                let dispatch_hash = self
                    .append(
                        EventBody::Dispatched {
                            id,
                            kind: req.kind.clone(),
                            family: req.content_type.family.as_ref().into(),
                            target: req.target.clone(),
                            idempotency_key,
                            deadline_ms: None,
                            token,
                        },
                        Some(cause),
                    )
                    .await;
                let outcome = if self.persist_error.is_some() {
                    // S1 latch: an un-durable dispatch never mutates the store (same tier-B rule the routed
                    // path applies before executing) — the set/resolve is NOT applied.
                    EffectOutcome::err(
                        "dispatch not durably logged (persist failure) — store effect NOT applied (S1)"
                            .to_string(),
                    )
                } else if crate::effect::effect_ct::is_group_store_family(&req.content_type.family)
                {
                    // §4c I3b GROUP sub-partition: store/add|remove|resolve-all act on OR-set groups (member-op
                    // payload) — routed to the group handler, distinct from the single-value pointer path.
                    self.apply_group_store_effect(&req, idempotency_key)
                } else {
                    self.apply_store_effect(&req, idempotency_key)
                };
                let more = self
                    .record_result(id, outcome, reducer, dispatch_hash)
                    .await;
                // The EffectResult is now recorded IN THE IN-MEMORY SESSION LOG (durability is separate — the
                // write-through `append` may have latched a persist_error; it is NOT guaranteed on disk here)
                // → this store effect is SETTLED in this session, so its dedup key can be pruned to BOUND
                // applied_set_keys to the in-flight window (liaison #1852: unbounded otherwise → memory/DoS).
                // Why pruning is re-drive-SAFE even on a persist-latched path (liaison #1858): (a) recovery
                // re-attaches a FRESH NameStore (the store is external state, NOT rebuilt from the session log
                // — see the `name_store` field), so a post-crash re-drive re-applies into an EMPTY store, which
                // is correct, not a duplicate; (b) an in-process re-drive of this id is blocked by `settled`
                // (record_result early-returns on a settled id). So the pruned key can't re-open the #1844
                // re-apply. (A resolve's key was never inserted → the prune is a no-op.)
                if let Some(store) = self.name_store.as_mut() {
                    store.forget_applied_key(&idempotency_key);
                }
                for pair in more {
                    to_process.push(pair);
                }
                continue;
            }

            // Timers arm a kernel-fired deadline (§9c), not an executor call. Keyed on the content-type
            // FAMILY (seq-39), not the legacy kind enum: the kernel routes by family.
            if req
                .content_type
                .matches_family(crate::effect::effect_ct::TIMER)
            {
                match req.target.parse::<u64>() {
                    Ok(deadline_ms) => {
                        self.append(
                            EventBody::TimerArmed {
                                id,
                                deadline_ms,
                                token,
                            },
                            Some(cause),
                        )
                        .await;
                    }
                    Err(_) => {
                        let denial_hash = self
                            .append(
                                EventBody::AuthzDenied {
                                    id,
                                    reason: format!(
                                        "timer deadline not a u64 ms: {:?}",
                                        req.target
                                    ),
                                    token,
                                },
                                Some(cause),
                            )
                            .await;
                        for pair in self.fold_tip(reducer, denial_hash).await {
                            to_process.push(pair);
                        }
                    }
                }
                continue;
            }

            let idempotency_key = idempotency_key_for(id, &req);

            // S1: durable dispatch record BEFORE routing.
            let dispatch_hash = self
                .append(
                    EventBody::Dispatched {
                        id,
                        kind: req.kind.clone(),
                        family: req.content_type.family.as_ref().into(),
                        target: req.target.clone(),
                        idempotency_key,
                        deadline_ms: None,
                        token,
                    },
                    Some(cause),
                )
                .await;

            // S1 latch-check BEFORE routing (tier B): an un-durable dispatch is NOT routed.
            if self.persist_error.is_some() {
                let outcome = EffectOutcome::err(
                    "dispatch not durably logged (persist failure) — effect NOT routed (S1)"
                        .to_string(),
                );
                let more = self
                    .record_result(id, outcome, reducer, dispatch_hash)
                    .await;
                for pair in more {
                    to_process.push(pair);
                }
                continue;
            }

            // Route + execute — awaiting the async executor (a real I/O executor yields here without
            // blocking the single-threaded loop). The Dispatched record is already durable, so ordering
            // is preserved across the await. DEBUG-trace only NON-SENSITIVE metadata: `target` is
            // guest-controlled (may carry a secret/PII) and `family` is guest-controllable for an extension
            // family, and tracing ships off-box — so `target`→len, `family`→`loggable_family` (static name
            // or "<extension>")+len, never the raw guest bytes (#2169/#2180). The full target is in the
            // durable Dispatched event above, which stays on-box.
            debug!(
                effect_id = id.0,
                family = loggable_family(&req.content_type.family),
                family_len = req.content_type.family.len(),
                target_len = req.target.len(),
                "dispatching effect to executor"
            );
            let outcome = executor.perform(&req, idempotency_key).await;

            // MONOTONIC `now` clamp (operator ruling): only `now` results are clamped. Keyed on the
            // content-type FAMILY (seq-39), not the legacy kind enum.
            let outcome = if req
                .content_type
                .matches_family(crate::effect::effect_ct::NOW)
            {
                clamp_now_outcome(outcome, &mut self.last_now)
            } else {
                outcome
            };

            let more = self
                .record_result(id, outcome, reducer, dispatch_hash)
                .await;
            for pair in more {
                to_process.push(pair);
            }
        }
        control_out
    }

    /// Fold the tip through a [`Reducer`] (`.await`), with FoldFailed capture + effect-reversal. This is
    /// the ONE place the reducer actually awaits.
    async fn fold_tip(&mut self, reducer: &dyn Reducer, cause: Hash) -> Vec<(Effect, Hash)> {
        let tip = self.log.last().expect("log always has genesis").clone();
        let out = reducer.fold(&tip, &mut self.kv).await;
        // Error-resilience (§17): a failed fold is captured as a FoldFailed log event, not folded further.
        if let Some(reason) = out.failure {
            // A fold failure (guest trap / fuel-exhaustion / instantiate error) — WARN, since it's an
            // error-resilience event a supervisor watches for (the loop can't be bricked, §17, but the
            // fold produced nothing). `reason` is the host's classification string, not guest data.
            warn!(%reason, "reducer fold failed — captured as FoldFailed (no effects this turn)");
            self.append(
                EventBody::FoldFailed {
                    reason,
                    caused_event: cause,
                },
                Some(cause),
            )
            .await;
            return Vec::new();
        }
        let mut v: Vec<(Effect, Hash)> = out.effects.into_iter().map(|e| (e, cause)).collect();
        v.reverse();
        v
    }

    /// Record an effect's result + fold it through a [`Reducer`] (`.await`), with timeout-cancels (drop a
    /// late result for an already-settled id) + the token-copy-from-Dispatched-frame invariant.
    async fn record_result(
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
        let result_hash = self
            .append(
                EventBody::EffectResult {
                    id,
                    result: outcome,
                    token,
                },
                Some(dispatch_hash),
            )
            .await;
        self.fold_tip(reducer, result_hash).await
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
        // §lifecycle I1: a TERMINATED session times out nothing — the terminal marker must stay the log tail
        // (a timeout `EffectResult` appended after it would un-tail the marker + flip is_terminated() back to
        // false). Return false (no-op), mirroring the deliver_control/fire_due_timers refusals. (github-liaison
        // #2381 review: guard EVERY append/drive entry point, not just deliver_control.)
        if self.is_terminated() {
            return false;
        }
        // Idempotent: only an OPEN id can be timed out. Settled (or never-dispatched) → no-op, so a late
        // real result and a timeout can't both settle one id (§16c-S4 at-most-once).
        if !self.open.contains(&id.0) {
            return false;
        }
        // `open` holds BOTH dispatched-effect ids AND armed-timer ids — but only a DISPATCHED effect can
        // be timed out (a timer isn't a hung external call; it fires via `fire_due_timers`). A timer
        // id has no `Dispatched` event, so `dispatch_hash_of` is None → return false (Copilot PR#1016: the
        // old code panicked here, contradicting the "never dispatched → false" contract). Timing out a
        // timer is a no-op, not a crash.
        let Some(dispatch_hash) = self.dispatch_hash_of(id) else {
            return false;
        };
        // Link the timeout result to the dispatch that opened it (causal DAG §5), like a real result.
        let more = self
            .record_result(id, EffectOutcome::TimedOut, reducer, dispatch_hash)
            .await;
        // The reducer's timeout continuation may emit further effects — drive them to quiescence.
        self.drive_worklist(more, reducer, authz, executor).await;
        true
    }

    /// Settle a fold-back control effect (`control/signature`) with the HOST's answer — the missing half of
    /// the control-plane fold-back contract. When the drive loop surfaces a fold-back control family
    /// ([`crate::effect::effect_ct::is_fold_back_control`]) it gives the effect a `Dispatched` frame and hands
    /// the driver a [`ControlEffect`](crate::effect::ControlEffect) carrying its [`EffectId`]; the host
    /// produces the answer off-band (e.g. reflecting the target component's signature — wasmtime, host-side)
    /// and calls this to fold it back into the EMITTING reducer's continuation. The result is a logged
    /// `EffectResult` causally linked to the `Dispatched` frame and keyed by its continuation token —
    /// identical to how any routed effect (shell/http) settles — so the guest resumes exactly where it awaited
    /// the query, and live-kv == replayed-kv (§9d). The reducer's continuation may emit further effects; they
    /// are driven to quiescence here.
    ///
    /// Idempotent + at-most-once, exactly like [`Session::time_out_effect`] (whose shape this mirrors): a
    /// TERMINATED session settles nothing (the terminal marker must stay the log tail) → `false`; an `id` that
    /// is not OPEN — already settled by a prior call, timed out, or never a fold-back dispatch — is a no-op
    /// `false`, so a late or duplicate host settle can never append a second `EffectResult` for one id (a
    /// continuation resumes at most once). Returns `true` iff this call settled it. `outcome` is the host's
    /// answer: `Ok(Some(Inline(descriptor)))` on success, or `EffectOutcome::err(..)` on a reflect/produce
    /// failure (the reducer folds the error and resumes cleanly, same as a failed routed effect).
    pub async fn settle_control_result(
        &mut self,
        id: EffectId,
        outcome: EffectOutcome,
        reducer: &dyn Reducer,
        authz: &(impl Authorize + ?Sized),
        executor: &mut (impl Executor + ?Sized),
    ) -> bool {
        // A TERMINATED session settles nothing — appending an EffectResult after the terminal marker would
        // un-tail it + flip is_terminated() back to false (same guard as time_out_effect/deliver_control).
        if self.is_terminated() {
            return false;
        }
        // Idempotent: only an OPEN id can be settled. Settled/never-dispatched → no-op, so a late or duplicate
        // host settle and any other settler (a timeout) can't both settle one id (at-most-once, §16c-S4).
        if !self.open.contains(&id.0) {
            return false;
        }
        // A fold-back control effect was opened by a `Dispatched` frame (like a routed effect), so it HAS a
        // dispatch hash. An `id` in `open` with no `Dispatched` (an armed TIMER) is not settleable here →
        // no-op `false`, mirroring time_out_effect's timer guard (never a crash).
        let Some(dispatch_hash) = self.dispatch_hash_of(id) else {
            return false;
        };
        let more = self
            .record_result(id, outcome, reducer, dispatch_hash)
            .await;
        // The reducer's continuation (now resumed with the query answer) may emit further effects — drive
        // them to quiescence, same as the routed-result and timeout paths.
        self.drive_worklist(more, reducer, authz, executor).await;
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

    /// Apply a `store/*` effect to the attached mutable-name store and map the result to the
    /// [`EffectOutcome`] the drive loop folds back (§4c slice 3b). The effect's `target` is the mutable
    /// NAME; for `store/set` the payload is an inline `name-set` blob ([`crate::event_ast::encode_name_set`])
    /// carrying the hash, for `store/resolve` there is no payload. Every failure is an observable
    /// `EffectOutcome::Err` (§9d anti-stuck) — no store attached, a malformed payload, an Unscoped name, a
    /// never-set resolve — never a panic. A successful `store/resolve` returns the frozen hash as an inline
    /// `name-set`-shaped payload (name + resolved hash), so the reducer reads it back through the SAME §9b
    /// codec it would use for a set.
    fn apply_store_effect(&mut self, req: &EffectRequest, idempotency_key: Hash) -> EffectOutcome {
        let family = req.content_type.family.as_ref();
        let name = req.target.as_ref();
        // A `store/set` carries the target hash in an inline `name-set` payload; `store/resolve` carries
        // none. Decode → the optional hash apply_effect expects. A set with a missing/garbage payload is a
        // malformed effect (observable Err), not a panic. VALIDATE the payload's embedded name against the
        // effect TARGET: the target is what the authorizer gated (SEC-F1), so a set whose payload names a
        // DIFFERENT name than it was authorized for must be rejected — never silently write the payload name.
        let is_set = family == crate::effect::effect_ct::STORE_SET;
        let hash = match &req.payload {
            Some(crate::effect::Payload::Inline(bytes)) => {
                match crate::event_ast::decode_name_set(bytes) {
                    Ok((payload_name, h)) => {
                        // The name==target check is a store/SET concern only (the target is what authz gated
                        // for the set); a resolve carries no name-set to validate — apply_effect rejects a
                        // resolve-with-payload as MalformedStoreEffect below, so don't apply the set-specific
                        // name-mismatch error to a non-set family.
                        if is_set && payload_name != name {
                            return EffectOutcome::err(format!(
                                "store/set payload name {payload_name:?} != authorized target {name:?} \
                                 — refusing (the target is what authz gated)"
                            ));
                        }
                        Some(h)
                    }
                    Err(e) => {
                        return EffectOutcome::err(format!(
                            "store effect: malformed name-set payload: {e:?}"
                        ));
                    }
                }
            }
            Some(crate::effect::Payload::Blob(_)) => {
                return EffectOutcome::err(
                    "store effect: blob-ref payload unsupported — inline the name-set".to_string(),
                );
            }
            None => None,
        };
        let Some(store) = self.name_store.as_mut() else {
            return EffectOutcome::err(
                "store effect: no name store attached to this session (attach_name_store)"
                    .to_string(),
            );
        };
        match store.apply_effect(family, name, hash, idempotency_key) {
            Ok(crate::name_store::StoreOutcome::Set(_)) => {
                // A set's outcome is empty-success — the reducer keyed the continuation by EffectId; the
                // set's value already rode the request payload. (A future slice may echo the new value.)
                EffectOutcome::Ok(None)
            }
            Ok(crate::name_store::StoreOutcome::Resolved(h)) => {
                // Return the resolved hash in the SAME name-set codec shape (name + hash) the guest decodes.
                let bytes = crate::event_ast::encode_name_set(name, &h);
                EffectOutcome::Ok(Some(crate::effect::Payload::Inline(bytes.into())))
            }
            // GROUP outcomes (GroupOpApplied / Members) can only come from `apply_group_effect`, which this
            // POINTER path never calls — the drive loop routes group verbs to a separate arm (§4c I3b, not yet
            // wired). A group outcome here would mean a mis-route; fold it as an observable Err (anti-stuck),
            // never a panic, until the group drive-loop arm lands.
            Ok(
                crate::name_store::StoreOutcome::GroupOpApplied
                | crate::name_store::StoreOutcome::Members(_),
            ) => EffectOutcome::err(format!(
                "store effect on {name:?}: a group OR-set outcome from the pointer path \
                 (misroute — group verbs use apply_group_effect)"
            )),
            Err(e) => EffectOutcome::err(format!("store effect on {name:?}: {e:?}")),
        }
    }

    /// Apply a GROUP `store/*` effect (§4c session-directory I3b: `store/add` / `store/remove` /
    /// `store/resolve-all`) to the attached name-store — the OR-set counterpart to [`apply_store_effect`]. The
    /// effect's `target` is the mutable GROUP NAME; for add/remove the `(member, tag)` rides the payload as a
    /// `member-op` blob ([`crate::event_ast::encode_member_op`] — the SAME frame the durable snapshot uses, so
    /// one codec spans the wire and the store), for `resolve-all` there is no payload. A successful
    /// `resolve-all` returns the frozen membership as a `members` blob ([`crate::event_ast::encode_members`],
    /// ascending-hash → byte-stable), which the guest decodes the same way a snapshot reader would; add/remove
    /// return empty-success (the op already rode the payload). Every failure — no store attached, a malformed/
    /// wrong-shaped payload, a mode-mismatch, an Unscoped or never-touched group — is an observable
    /// `EffectOutcome::Err` (§9d anti-stuck), never a panic.
    fn apply_group_store_effect(
        &mut self,
        req: &EffectRequest,
        idempotency_key: Hash,
    ) -> EffectOutcome {
        let family = req.content_type.family.as_ref();
        let name = req.target.as_ref();
        // add/remove carry a `member-op` payload (member + tag); resolve-all carries none. Decode → the
        // optional MemberOp apply_group_effect expects. VALIDATE the payload's embedded name against the effect
        // TARGET (the target is what the authorizer gated, SEC-F1): an op whose payload names a DIFFERENT group
        // than it was authorized for must be rejected, never silently applied to the payload name.
        let op = match &req.payload {
            Some(crate::effect::Payload::Inline(bytes)) => {
                match crate::event_ast::decode_member_op(bytes) {
                    Ok((payload_name, add, member, tag)) => {
                        if payload_name != name {
                            return EffectOutcome::err(format!(
                                "group store effect payload name {payload_name:?} != authorized target \
                                 {name:?} — refusing (the target is what authz gated)"
                            ));
                        }
                        Some(crate::name_store::MemberOp { add, member, tag })
                    }
                    Err(e) => {
                        return EffectOutcome::err(format!(
                            "group store effect: malformed member-op payload: {e:?}"
                        ));
                    }
                }
            }
            Some(crate::effect::Payload::Blob(_)) => {
                return EffectOutcome::err(
                    "group store effect: blob-ref payload unsupported — inline the member-op"
                        .to_string(),
                );
            }
            None => None,
        };
        let Some(store) = self.name_store.as_mut() else {
            return EffectOutcome::err(
                "group store effect: no name store attached to this session (attach_name_store)"
                    .to_string(),
            );
        };
        match store.apply_group_effect(family, name, op, idempotency_key) {
            Ok(crate::name_store::StoreOutcome::GroupOpApplied) => {
                // An add/remove's outcome is empty-success — the op's member+tag already rode the payload; the
                // reducer keyed its continuation by EffectId.
                EffectOutcome::Ok(None)
            }
            Ok(crate::name_store::StoreOutcome::Members(members)) => {
                // resolve-all: return the frozen membership as a `members` blob (ascending-hash → byte-stable),
                // decoded the same way a snapshot reader decodes group state.
                let bytes = crate::event_ast::encode_members(&members);
                EffectOutcome::Ok(Some(crate::effect::Payload::Inline(bytes.into())))
            }
            // POINTER outcomes (Set/Resolved) can't come from apply_group_effect (a group family never
            // dispatches to the pointer verbs); fold as an observable Err if one somehow arrives (anti-stuck,
            // no panic) — the mirror of the group-outcome guard on the pointer path.
            Ok(
                crate::name_store::StoreOutcome::Set(_)
                | crate::name_store::StoreOutcome::Resolved(_),
            ) => EffectOutcome::err(format!(
                "group store effect on {name:?}: a single-value outcome from the group path (misroute)"
            )),
            Err(e) => EffectOutcome::err(format!("group store effect on {name:?}: {e:?}")),
        }
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

    /// The effect FAMILY that dispatch `id`'s `Dispatched` frame recorded, or `None` if `id` has no
    /// `Dispatched` frame (e.g. a timer, opened by `TimerArmed`). Used on replay to tell a `now` result
    /// apart (so `last_now` rebuilds only from `now` results). Keys on the durable `family` string (seq-39,
    /// the authoritative identity) rather than the legacy `kind` enum, so it stays correct for a
    /// register-by-string family with no `EffectKind` variant. Reads the durable frame → replay-deterministic.
    fn dispatch_family_of(&self, id: EffectId) -> Option<std::sync::Arc<str>> {
        // Scan from the END (rev): at most ONE matching frame per id, and a result/fire event is
        // near its dispatch/arm, so the reverse scan finds it fast — avoids an O(log^2) replay hot
        // path where a front scan re-walks the whole prefix for every EffectResult (PR#1253 review).
        self.log.iter().rev().find_map(|e| match &e.body {
            EventBody::Dispatched { id: d, family, .. } if *d == id => Some(family.clone()),
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

    /// Reconstruct a session from a persisted log, folding each observable event through a [`Reducer`]
    /// (`.await`) and rebuilding the obligation sets / armed-timer table / `next_effect_id` / `last_now`
    /// high-water mark.
    ///
    /// Effects emitted during replay are IGNORED (§17 "replay re-folds with no live effect" — the results
    /// are already in the log); so replayed-kv == live-kv (PR#990 finding #1).
    pub async fn replay(log: Vec<Event>, reducer: &dyn Reducer) -> Result<Session, KernelError> {
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
            name_store: None,
            last_manifest: None,
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
                    if s.dispatch_family_of(*id).as_deref() == Some(crate::effect::effect_ct::NOW) {
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
                let _ = reducer.fold(&event, &mut s.kv).await;
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

    /// Boot a session from an ALREADY-READ recovery result — the backend-AGNOSTIC recovery core (§16c-S1).
    /// This is the real recovery logic: it takes a [`crate::log_store::Recovered`] (the good prefix of
    /// events + how the read ended) from ANY backend — a disk [`crate::log_store::LogStore`], a network /
    /// replicated log, an in-memory fixture — and folds it through [`Session::replay`] to reconstruct KV +
    /// the open-obligation set, returning the session together with a [`RecoveryReport`]. The kernel does
    /// NOT assume the log is backed by a file (operator directive: "the log should be generic"); reading the
    /// bytes is the backend's job, this is the fold-and-report job. [`Session::recover`] is the file
    /// convenience that reads via `LogStore` and calls this.
    ///
    /// - `kind` — how the log ended: `Clean`, `TornTail` (benign crash mid-append), or `Corrupt`
    ///   (a fully-present frame that didn't decode — an ALARM the driver must not miss; PR#993 #1
    ///   propagates this from the backend so callers can react, not just internal code). The session is
    ///   recovered to the last *whole* event before the tail either way.
    /// - `open_effects` — dispatched-but-unsettled effects the driver must re-drive (by their stable
    ///   idempotency key, so re-drive dedups rather than double-fires) or time out.
    ///
    /// Corruption is reported (not turned into a hard `Err`) so the driver keeps the recovered
    /// good-prefix + open-effects and DECIDES whether to proceed or halt — `report.kind` /
    /// `report.is_corrupt()` is the signal. An empty recovery (no events) is NOT recoverable as a session
    /// — the caller must `genesis()` a new one — reported as [`RecoverError::EmptyLog`].
    pub async fn recover_from(
        recovered: crate::log_store::Recovered,
        reducer: &dyn Reducer,
    ) -> Result<(Session, RecoveryReport), RecoverError> {
        if recovered.events.is_empty() {
            return Err(RecoverError::EmptyLog);
        }
        let kind = recovered.kind;
        let session = Session::replay(recovered.events, reducer)
            .await
            .map_err(RecoverError::Replay)?;
        let report = RecoveryReport {
            kind,
            open_effects: session.open_effect_ids(),
        };
        Ok((session, report))
    }

    /// Boot a session from a persisted log ON DISK — the file convenience over [`Session::recover_from`].
    /// Reads the durable log via [`crate::log_store::LogStore::recover`], then hands the [`crate::log_store::Recovered`] to
    /// the backend-agnostic core. Callers on a non-file backend read their own `Recovered` and call
    /// [`Session::recover_from`] directly, so the kernel core carries no file assumption.
    ///
    /// An empty/absent file is reported as [`RecoverError::EmptyLog`] (the caller must `genesis()` a fresh
    /// session instead); see [`Session::recover_from`] for the `kind`/`open_effects` report contract.
    pub async fn recover(
        path: impl AsRef<std::path::Path>,
        reducer: &dyn Reducer,
    ) -> Result<(Session, RecoveryReport), RecoverError> {
        let recovered = crate::log_store::LogStore::recover(path).map_err(RecoverError::Io)?;
        Session::recover_from(recovered, reducer).await
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
/// A tracing-SAFE rendering of an effect's content-type `family` (github-liaison #2180 residual): a
/// well-known static family (a built-in effect verb / exact `control/*` / exact `store/*`) is a fixed
/// kernel-defined string, safe to emit into a span/event; an EXTENSION family carries the guest's own
/// `Cow::Owned` bytes, so emitting it verbatim would leak guest-controlled data off-box via the tracing
/// subscriber (the same class as the effect `target`, redacted by #2180). For an extension family we
/// return a fixed marker instead of the bytes — the drive loop pairs it with the family LENGTH so the log
/// still carries a diagnostic signal (a well-known name, or `<extension>` + length) WITHOUT leaking guest
/// bytes. (Two distinct extension families both render `<extension>`, so this identifies the well-known set
/// exactly and marks the rest as opaque — it does not distinguish one extension family from another; that's
/// the point.) Gated on the EXACT static vocabulary
/// ([`crate::effect::effect_ct::wellknown_static_str`]), NOT a prefix check (a guest can craft
/// `store/<secret>` inside the "trusted" namespace).
fn loggable_family(family: &str) -> &'static str {
    crate::effect::effect_ct::wellknown_static_str(family).unwrap_or("<extension>")
}

/// Authorize a `shell` effect that carries a structured `(shell-pipeline …)` payload STAGE-BY-STAGE
/// (operator security directive: each pipeline stage's program is Cedar-authorized; deny the whole
/// pipeline if ANY stage is denied). Mechanism only — the WHICH-programs-allowed decision stays entirely
/// in `authz`; this just resolves each stage's program to a target and fans the EXISTING authz seam over
/// the stages, so the [`Authorize`] signature is unchanged.
///
/// Returns:
/// - `None` — this effect is NOT a shell-pipeline: a non-`shell` family, a payload-less shell (a bare
///   `target` command), a blob-ref payload (the drive loop has no blob store to resolve it), or an inline
///   payload that does NOT decode as a `(shell-pipeline …)` (e.g. an M3 tool-call's opaque raw input). The
///   caller falls back to the ordinary single-target `authorize(&req)`, so today's single-command shell +
///   every other effect are unchanged. The HOST must use this SAME "decodes as `(shell-pipeline …)`"
///   discriminant, so a guest can't get multi-stage execution without a decodable pipeline.
/// - `Some(Ok(()))` — the payload IS a pipeline and EVERY stage's program is permitted.
/// - `Some(Err(reason))` — the payload IS a pipeline but a stage's program is denied (deny-all: the FIRST
///   denied stage fails the whole pipeline) OR the pipeline is malformed for authorization (empty, or a
///   stage with an empty program). Never panics.
///
/// A synthetic per-stage [`EffectRequest`] carries the stage's program as its `target` (the SEC-F1 unit)
/// under the same `shell` family, no payload (a stage is a leaf program, not itself a pipeline).
async fn authorize_shell_pipeline(
    req: &EffectRequest,
    authz: &(impl Authorize + ?Sized),
) -> Option<Result<(), String>> {
    // Only a `shell` effect with an INLINE payload can be a pipeline. Everything else → None (single-target).
    if !req
        .content_type
        .matches_family(crate::effect::effect_ct::SHELL)
    {
        return None;
    }
    let bytes = match &req.payload {
        Some(crate::effect::Payload::Inline(bytes)) => bytes,
        // A blob-ref (no blob store to resolve here) or no payload → not a decodable pipeline → single-target.
        _ => return None,
    };
    // The DISCRIMINANT: does the payload decode as a `(shell-pipeline …)`? A non-pipeline inline payload
    // (e.g. an opaque tool-call input) fails the codec's head/shape check → None → the bare target gates.
    let pipeline = crate::event_ast::decode_shell_pipeline(bytes).ok()?;
    if pipeline.stages.is_empty() {
        return Some(Err(
            "shell pipeline: empty pipeline (no stages to authorize)".to_string(),
        ));
    }
    for (i, stage) in pipeline.stages.iter().enumerate() {
        if stage.program.is_empty() {
            return Some(Err(format!(
                "shell pipeline: stage {i} has an empty program"
            )));
        }
        // Each stage's PROGRAM is the resolved target of one authz call (the SEC-F1 unit). Same `shell`
        // family, no payload (a leaf program). Deny-all: the FIRST denied stage fails the whole pipeline.
        let stage_req = EffectRequest::new(
            EffectKind::Shell,
            stage.program.as_str(),
            None,
            req.timeliness.clone(),
        );
        if let Err(reason) = authz.authorize(&stage_req).await {
            return Some(Err(format!(
                "shell pipeline: stage {i} program denied — {reason}"
            )));
        }
    }
    Some(Ok(()))
}

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
        EventBody::Terminated { .. } => "Terminated",
        EventBody::Spawned { .. } => "Spawned",
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
                return EffectOutcome::err(
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
        // Terminated is NOT folded either — it's a terminal marker; a terminated session refuses all
        // further folds (the deliver-time FoldRefused guard), so its own marker is never handed to a
        // reducer (§lifecycle I1).
        // Spawned is a recorded parent→child edge (§I2, supervision-tree substrate), NOT a fold input —
        // like FoldFailed/Terminated, a supervisor reads it from the log; it's never handed to the reducer.
        EventBody::Genesis { .. }
        | EventBody::Dispatched { .. }
        | EventBody::TimerArmed { .. }
        | EventBody::FoldFailed { .. }
        | EventBody::Terminated { .. }
        | EventBody::Spawned { .. } => false,
    }
}

/// Derive a dispatch's idempotency key (§16c-S1). For v0 it's the hash of `(id, family, target)` — stable
/// across a re-drive of the *same* dispatch, distinct across different effects. A real side-effecting
/// executor dedups on this so a crash-recovery re-drive doesn't double-apply.
///
/// Keys on the content-type FAMILY string (seq-39), not the legacy `EffectKind` enum tag: a
/// register-by-string family carries the `Emit` placeholder kind, so two DISTINCT families (an extension
/// family and a real `emit`) would hash the SAME enum tag and could collide their keys — the family string
/// is the authoritative identity, so it can't. The key is an OPAQUE dedup handle (executors never pin a
/// specific value), so deriving it from family rather than the enum tag is a safe internal change; re-drive
/// consistency is preserved (same request → same family → same key). A length prefix separates the family
/// from the target so `(family="a", target="bc")` and `(family="ab", target="c")` can't alias.
fn idempotency_key_for(id: EffectId, req: &EffectRequest) -> Hash {
    let mut buf = Vec::new();
    buf.extend_from_slice(&id.0.to_le_bytes());
    let family = req.content_type.family.as_bytes();
    buf.extend_from_slice(&(family.len() as u64).to_le_bytes());
    buf.extend_from_slice(family);
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
        async fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
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

    // A reducer that FAILS its fold on an inbound (returns FoldOutput::failed) — models a wasm guest trap /
    // fuel-exhaustion surfaced as data (§17). Used to pin the FoldFailed error-capture path (§6a gap #1).
    struct FailingReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for FailingReducer {
        async fn fold(&self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    FoldOutput::failed("wasm reducer trapped: unreachable")
                }
                _ => FoldOutput::none(),
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_failed_fold_is_captured_as_a_foldfailed_event_and_the_session_survives() {
        // §6a error-resilience gap #1 (the supervision FOUNDATION): a fold that FAILS must be CAPTURED as a
        // first-class FoldFailed LOG event a supervisor can observe — NOT vanish into a silent empty fold
        // ("errors into the void"), NOT panic the loop, NOT wedge the session. Pins the drive-loop BEHAVIOR
        // (the codec round-trip is pinned separately in event.rs/event_ast).
        let mut exec = RecordingExecutor::new();
        let mut s = Session::genesis(Hash::of(b"fail-v1"), Hash::of(b"test-spawn-nonce"));
        s.deliver(
            inbound(),
            None,
            &FailingReducer,
            &Authorizer::deny_all(),
            &mut exec,
        )
        .await
        .unwrap(); // deliver itself SUCCEEDS — the fold failure is data, not a kernel error.

        // A FoldFailed event is on the log, carrying the reason + BOTH cause linkages: the body's
        // `caused_event` field AND the envelope `Event.cause` edge (distinct — the body field is a payload,
        // `Event.cause` is the real causal-DAG parent edge replay/tamper-evidence/consumers walk; a regression
        // that filled one but not the other would break the DAG, so pin BOTH — liaison pr1963).
        let (reason, body_caused, envelope_cause) = s
            .log()
            .iter()
            .find_map(|e| match &e.body {
                EventBody::FoldFailed {
                    reason,
                    caused_event,
                } => Some((reason.clone(), *caused_event, e.cause)),
                _ => None,
            })
            .expect("a failed fold records a FoldFailed event");
        assert_eq!(
            reason, "wasm reducer trapped: unreachable",
            "the failure reason is preserved"
        );
        // The inbound the fold choked on is on the log; FoldFailed links to it BOTH ways.
        let inbound_hash = s
            .log()
            .iter()
            .find_map(|e| matches!(&e.body, EventBody::Inbound { .. }).then(|| e.hash()))
            .expect("the inbound is logged");
        assert_eq!(
            body_caused, inbound_hash,
            "FoldFailed body caused_event names the event whose fold failed"
        );
        assert_eq!(
            envelope_cause,
            Some(inbound_hash),
            "FoldFailed's ENVELOPE Event.cause edge points at the inbound too (the causal-DAG edge, not just \
             the body field) — replay/tamper-evidence walk this"
        );

        // No effects were routed (a failed fold carries none), and the session is NOT stuck — it's a normal
        // observable state a supervisor reads, and the loop didn't panic.
        assert!(exec.seen.is_empty(), "a failed fold routes no effects");
        assert_eq!(
            s.open_effects(),
            0,
            "no dispatched-but-unsettled obligations from a failed fold"
        );

        // Self-heal precondition: ONE failed fold doesn't wedge the session — a SUBSEQUENT deliver still folds.
        // (StatusReducer here just to prove the session accepts + processes a new event after the failure.)
        s.deliver(inbound(), None, &StatusReducer, &timer_cap(), &mut exec)
            .await
            .unwrap();
        assert!(
            s.kv().get(b"public/status").is_some(),
            "the session survives a failed fold and processes the next event"
        );
    }

    // A report-aware reducer (the fork-for-query summarize protocol, operator ruling (a)): on an ordinary
    // message it does work + publishes status; on the well-known `report` content-type it describes ITSELF
    // from LOCAL STATE (no model call — the operator's preferred path) by writing a summary to `public/`.
    // This is the generic-reducer shape a query fold uses: `if ct.is_report() { …summarize… }`.
    struct ReportingReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for ReportingReducer {
        async fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
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

    // CROSS-SESSION MESSAGING (operator's next big rock): an "emitter" reducer that, on an inbound trigger,
    // performs ONE real Emit effect targeting a PEER session id with a message payload — the sender half of
    // A.Emit → host-routes → B.Inbound. This pins the KERNEL contract the joint host E2E (v-agent-harness-host
    // owns the routing + 2-session harness) leans on: a reducer's Emit(target=<peer id>) is authorized against
    // an Emit capability over that target, then recorded as a durable Dispatched frame carrying kind=Emit +
    // the peer target (what the host's EmitExecutor routes from). Distinct from the CONTROL-family Emit
    // placeholder tests above — this is a REAL peer-directed emit (family `emit`, a concrete target).
    struct PeerEmitterReducer {
        peer: &'static str,
    }
    #[async_trait::async_trait(?Send)]
    impl Reducer for PeerEmitterReducer {
        async fn fold(&self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => FoldOutput::with(vec![EffectRequest::new(
                    EffectKind::Emit,
                    self.peer,
                    Some(crate::effect::Payload::Inline(
                        b"hello-peer".to_vec().into(),
                    )),
                    Timeliness::Interactive,
                )]),
                _ => FoldOutput::none(),
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_peer_directed_emit_is_authorized_dispatched_and_routed_with_its_target() {
        // The kernel half of cross-session messaging: emitter folds → kernel AUTHORIZES the Emit against an
        // Emit-cap over the exact peer target → durably records a Dispatched{kind:Emit, target:<peer>} →
        // surfaces the effect to the executor (the host's EmitExecutor routes it to the peer's inbox). Assert
        // all three: the executor SAW the emit with the right target+payload; the Dispatched frame recorded
        // kind=Emit + the peer target; the emit was authorized (it wasn't dropped).
        let peer = "session-B";
        let mut exec = RecordingExecutor::new();
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Emit,
            predicate: ResourcePredicate::Exact(peer.into()),
        }]);
        let mut s = Session::genesis(Hash::of(b"emitter-v1"), Hash::of(b"test-spawn-nonce"));
        s.deliver(
            inbound(),
            None,
            &PeerEmitterReducer { peer },
            &authz,
            &mut exec,
        )
        .await
        .expect("deliver");

        // The executor saw exactly the peer-directed emit (this is what the host EmitExecutor routes).
        assert_eq!(
            exec.seen.len(),
            1,
            "the authorized emit reached the executor"
        );
        let (req, _key) = &exec.seen[0];
        assert!(matches!(req.kind, EffectKind::Emit), "kind is Emit");
        assert_eq!(req.target.as_ref(), peer, "target is the peer session id");
        assert_eq!(
            req.payload,
            Some(crate::effect::Payload::Inline(
                b"hello-peer".to_vec().into()
            )),
            "the message payload rides the emit"
        );

        // A durable Dispatched frame recorded kind=Emit + the peer target (the crash-recovery-safe record
        // the host routes from — before the effect leaves the kernel).
        let (kind, target) = s
            .log()
            .iter()
            .find_map(|e| match &e.body {
                EventBody::Dispatched { kind, target, .. } => Some((kind.clone(), target.clone())),
                _ => None,
            })
            .expect("a Dispatched frame was recorded for the emit");
        assert!(
            matches!(kind, EffectKind::Emit),
            "Dispatched records kind=Emit"
        );
        assert_eq!(target.as_ref(), peer, "Dispatched records the peer target");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_peer_emit_to_an_ungranted_target_is_denied_not_routed() {
        // Cedar gate on cross-session messaging: a session may Emit ONLY where its capability grants the
        // target. An emit to a peer the capability does NOT cover is DENIED — never surfaced to the executor
        // (so the host never routes it), never a Dispatched frame. Grant Emit to "session-B" but emit to
        // "session-C" → denied. (This is the kernel-side of the authz v-agent-harness-host gates §20b-side.)
        let mut exec = RecordingExecutor::new();
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Emit,
            predicate: ResourcePredicate::Exact("session-B".into()),
        }]);
        let mut s = Session::genesis(Hash::of(b"emitter-deny-v1"), Hash::of(b"test-spawn-nonce"));
        s.deliver(
            inbound(),
            None,
            &PeerEmitterReducer { peer: "session-C" }, // NOT the granted target
            &authz,
            &mut exec,
        )
        .await
        .expect("deliver");

        assert!(
            exec.seen.is_empty(),
            "an emit to an ungranted peer target must NOT reach the executor (Cedar-denied, so unroutable)"
        );
        assert!(
            !s.log()
                .iter()
                .any(|e| matches!(&e.body, EventBody::Dispatched { .. })),
            "a denied emit records NO Dispatched frame (it never dispatches)"
        );
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
        let s = Session::genesis(Hash::of(b"r"), Hash::of(b"test-spawn-nonce"));
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
        let mut s = Session::genesis(Hash::of(b"status-v1"), Hash::of(b"test-spawn-nonce"));
        s.deliver(inbound(), None, &StatusReducer, &timer_cap(), &mut exec)
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
        let mut s = Session::genesis(Hash::of(b"status-v1"), Hash::of(b"test-spawn-nonce"));
        s.deliver(inbound(), None, &StatusReducer, &timer_cap(), &mut exec)
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
    async fn deliver_is_deterministic_same_input_same_state() {
        // Determinism (§3): `deliver` is REPRODUCIBLE — same starting state + same inputs (inbound,
        // reducer, authz, executor) ⇒ BYTE-IDENTICAL result. Two independent sessions, both from a fresh
        // genesis, fed the same inbound through the same reducer/authz/executor must end identical. Build
        // two and assert they match on the KV ROOT HASH (the whole KV, not one key), the log length, and
        // the derived status. Any nondeterminism in the drive loop fails loudly HERE. (Historically this
        // pinned sync-vs-async equivalence; post all-async collapse there is one `deliver`, so it now pins
        // delivery determinism.)

        let mut sync_exec = RecordingExecutor::new();
        let mut sync_s = Session::genesis(Hash::of(b"status-v1"), Hash::of(b"test-spawn-nonce"));
        sync_s
            .deliver(
                inbound(),
                None,
                &StatusReducer,
                &timer_cap(),
                &mut sync_exec,
            )
            .await
            .unwrap();

        let mut async_exec = RecordingExecutor::new();
        let mut async_s = Session::genesis(Hash::of(b"status-v1"), Hash::of(b"test-spawn-nonce"));
        async_s
            .deliver(
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
    // so a test can prove fire_due_timers actually wakes the reducer (not just drains the table).
    struct TimerThenPublishReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for TimerThenPublishReducer {
        async fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
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
    async fn fire_due_timers_wakes_the_reducer_and_settles_the_timer() {
        // fire_due_timers is a new public API; pin it: arm a timer via an inbound, then fire it and
        // assert (a) it returns 1 (one timer fired), (b) the reducer WOKE (its TimerFired fold ran, writing
        // the marker), (c) the armed-timer + open-obligation sets drained (the timer settled). Determinism:
        // the recorded fired_ms is the timer's deadline, so replay is stable.
        let reducer = TimerThenPublishReducer;
        let mut exec = RecordingExecutor::new();
        let mut s = Session::genesis(Hash::of(b"timer-v1"), Hash::of(b"test-spawn-nonce"));
        s.deliver(inbound(), None, &reducer, &timer_cap(), &mut exec)
            .await
            .unwrap();
        // Armed, not yet fired.
        assert_eq!(s.next_timer_deadline(), Some(1000));
        assert!(s.kv().get(b"public/woke").is_none());

        // Fire everything due at now=1500 (past the 1000ms deadline).
        let fired = s
            .fire_due_timers(1500, &reducer, &timer_cap(), &mut exec)
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
        let mut s = Session::genesis(Hash::of(b"status-v1"), Hash::of(b"test-spawn-nonce"));
        s.deliver(inbound(), None, &StatusReducer, &timer_cap(), &mut exec)
            .await
            .unwrap();
        let parent_events = s.log().len();

        let mut fork = s.fork_for_query();
        let mut fork_exec = RecordingExecutor::new();
        fork.deliver(
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
        let mut live = Session::genesis(Hash::of(b"reporting-v1"), Hash::of(b"test-spawn-nonce"));
        // The live session does ordinary work: records a private goal + public status.
        live.deliver(inbound(), None, &ReportingReducer, &timer_cap(), &mut exec)
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
        fork.deliver(
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
        let mut s = Session::genesis(Hash::of(b"r"), Hash::of(b"test-spawn-nonce"));
        // Append a Closed event directly (a session that shut down).
        s.append(
            EventBody::Closed {
                outcome: crate::event::CloseOutcome::Success(crate::effect::Payload::Inline(
                    b"".to_vec().into(),
                )),
            },
            None,
        )
        .await;
        let snap = s.status_snapshot(Some(0), 300_000);
        assert_eq!(snap.state, SessionState::Closed);
        assert_eq!(snap.last_event_kind, "Closed");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_failure_close_still_reports_closed_state() {
        // The session STATE is outcome-AGNOSTIC: a session closed with CloseOutcome::Failure is just as
        // `Closed` as a Success-close (a supervisor observes "closed" from the state, then reads the
        // success-vs-failure from the outcome SEPARATELY — §6a). Pins that a Failure-close doesn't leave the
        // session looking Active/Quiescent (which would strand a supervisor waiting on a child that's done).
        let mut s = Session::genesis(Hash::of(b"r"), Hash::of(b"test-spawn-nonce"));
        s.append(
            EventBody::Closed {
                outcome: crate::event::CloseOutcome::Failure("goal unreachable".to_string()),
            },
            None,
        )
        .await;
        let snap = s.status_snapshot(Some(0), 300_000);
        assert_eq!(
            snap.state,
            SessionState::Closed,
            "a Failure-close reports Closed, same as a Success-close (state is outcome-agnostic)"
        );
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
            EffectOutcome::Err { message: msg, .. } => {
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
        let e = clamp_now_outcome(EffectOutcome::err("boom"), &mut last);
        assert!(matches!(e, EffectOutcome::Err { .. }));
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
        async fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
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
        async fn perform(&mut self, req: &EffectRequest, _key: Hash) -> EffectOutcome {
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
        let mut s = Session::genesis(Hash::of(b"now-v1"), Hash::of(b"test-spawn-nonce"));
        s.deliver(inbound(), None, &NowReducer, &now_cap(), &mut exec)
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
        let mut s = Session::genesis(Hash::of(b"now-v1"), Hash::of(b"test-spawn-nonce"));
        s.deliver(inbound(), None, &NowReducer, &now_cap(), &mut exec)
            .await
            .unwrap();
        let live_seq = recorded_now_sequence(&s);
        let live_last_now = s.last_now;

        // Replay the log: the recorded (already-clamped) Now results must rebuild the SAME last_now +
        // the SAME sequence — replay never re-clamps, it re-derives (determinism).
        let log = s.log().to_vec();
        let replayed = Session::replay(log, &NowReducer).await.expect("replay");
        assert_eq!(recorded_now_sequence(&replayed), live_seq);
        assert_eq!(replayed.last_now, live_last_now);
        assert_eq!(replayed.last_now, 1002);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn replay_reconstructs_kv_and_open_set_deterministically() {
        // `replay` reconstructs a session from its log DETERMINISTICALLY: replaying the same log yields the
        // same KV root, last_now, open set, and length as the live session that produced it — and two
        // replays of the same log agree (idempotent, no hidden nondeterminism in the re-fold). (Formerly a
        // sync-vs-async replay equivalence check; there is only ONE async replay now — the all-async arc
        // dropped the sync twin — so this pins single-replay determinism instead.)
        let mut exec = StuckClock(1000);
        let mut s = Session::genesis(Hash::of(b"now-v1"), Hash::of(b"test-spawn-nonce"));
        s.deliver(inbound(), None, &NowReducer, &now_cap(), &mut exec)
            .await
            .unwrap();
        let log = s.log().to_vec();

        let replayed = Session::replay(log.clone(), &NowReducer)
            .await
            .expect("replay");
        // Reconstructs the live session's derived state exactly.
        assert_eq!(replayed.snapshot().kv_root, s.snapshot().kv_root);
        assert_eq!(replayed.last_now, s.last_now);
        assert_eq!(replayed.last_now, 1002);
        assert_eq!(replayed.log().len(), s.log().len());
        assert_eq!(recorded_now_sequence(&replayed), recorded_now_sequence(&s));

        // Two replays of the same log agree — the re-fold has no hidden nondeterminism.
        let replayed2 = Session::replay(log, &NowReducer).await.expect("replay 2");
        assert_eq!(replayed2.snapshot().kv_root, replayed.snapshot().kv_root);
    }

    // A reducer that, on an inbound message, emits a `control/summary` effect carrying summary bytes in
    // its payload (the fork-for-query control-plane pattern). It's a control/* family → authz-exempt +
    // host-surfaced (register-by-string beat 3).
    struct SummaryEmitReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for SummaryEmitReducer {
        async fn fold(&self, event: &Event, _kv: &mut Kv) -> FoldOutput {
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
        // the kernel returns it to the driver via deliver_control. Prove all three: it's returned
        // (with its payload + token), the executor NEVER saw it, and a DENY-ALL authz did NOT deny it
        // (control is exempt — a normal effect under deny_all would be AuthzDenied).
        let mut exec = RecordingExecutor::new();
        let mut session = Session::genesis(Hash::of(b"control-v1"), Hash::of(b"test-spawn-nonce"));
        let control = session
            .deliver_control(
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

        // The common deliver path drops the control Vec but is otherwise identical (returns ()).
        let mut exec2 = RecordingExecutor::new();
        let mut s2 = Session::genesis(Hash::of(b"control-v2"), Hash::of(b"test-spawn-nonce"));
        s2.deliver(
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
        async fn fold(&self, event: &Event, _kv: &mut Kv) -> FoldOutput {
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
        let mut session = Session::genesis(Hash::of(b"mixed-v1"), Hash::of(b"test-spawn-nonce"));
        // Grant the emit family (any resource) so the REGULAR effect authorizes; control is exempt anyway.
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Emit,
            predicate: crate::effect::ResourcePredicate::Any,
        }]);
        let control = session
            .deliver_control(inbound(), None, &MixedEmitReducer, &authz, &mut exec)
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

    // A reducer that models the SIGNATURE-QUERY fold-back loop: on an inbound message it emits a
    // `control/signature` effect (targeting a component by hash/name) with a continuation token; when the
    // HOST's answer folds back as an EffectResult, it RESUMES by writing the returned descriptor bytes into KV
    // under `sig/descriptor`. This is the shape a real orchestration reducer uses — discover a target's
    // callable surface, then route a call — and it lets the test observe that the continuation actually
    // resumed with the host's answer (not just that an effect was surfaced).
    struct SignatureQueryReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for SignatureQueryReducer {
        async fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    let mut request = EffectRequest::new(
                        EffectKind::Emit, // kind irrelevant for a control family; the family drives routing
                        "component-hash-abc",
                        None,
                        Timeliness::Interactive,
                    );
                    request.content_type.family = crate::effect::effect_ct::SIGNATURE.into();
                    FoldOutput::with_effects(vec![Effect {
                        request,
                        token: Some(b"sig-cont".to_vec()),
                    }])
                }
                // The host's answer folded back → resume: record the descriptor the query returned.
                EventBody::EffectResult {
                    result: EffectOutcome::Ok(Some(crate::effect::Payload::Inline(bytes))),
                    ..
                } => {
                    kv.put(b"sig/descriptor".to_vec(), bytes.to_vec());
                    FoldOutput::none()
                }
                // A failed query → record the failure marker so the reducer resumes cleanly on the err path.
                EventBody::EffectResult {
                    result: EffectOutcome::Err { .. },
                    ..
                } => {
                    kv.put(b"sig/error".to_vec(), b"query-failed".to_vec());
                    FoldOutput::none()
                }
                _ => FoldOutput::none(),
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn control_signature_is_surfaced_dispatched_and_settle_control_result_resumes_the_guest()
    {
        // The fold-back control pattern (control/signature = the THIRD control disposition): unlike
        // capabilities (kernel-answered inline) or summary (fire-and-forget fork-scrape, no Dispatched), a
        // signature query is SURFACED to the driver AND given a Dispatched frame, so it is OPEN and awaiting a
        // HOST answer that must resume the emitting reducer. Prove the whole loop: surface → open+dispatched →
        // settle_control_result folds the descriptor back → the guest's continuation resumes (writes KV).
        let mut exec = RecordingExecutor::new();
        let mut session = Session::genesis(Hash::of(b"sig-v1"), Hash::of(b"nonce"));
        let control = session
            .deliver_control(
                inbound(),
                None,
                &SignatureQueryReducer,
                &Authorizer::deny_all(), // control is authz-EXEMPT — deny_all must not deny it
                &mut exec,
            )
            .await
            .expect("deliver");

        // Surfaced to the driver (host-answered, needs wasmtime reflection), NEVER routed to an executor.
        assert_eq!(
            control.len(),
            1,
            "the signature query surfaces to the driver"
        );
        assert_eq!(
            control[0].request.content_type.family.as_ref(),
            crate::effect::effect_ct::SIGNATURE
        );
        assert_eq!(control[0].token.as_deref(), Some(&b"sig-cont"[..]));
        assert_eq!(
            exec.seen.len(),
            0,
            "control is never routed to the executor"
        );
        let sig_id = control[0].id;

        // Unlike summary, a fold-back control got a Dispatched frame → it is OPEN + awaiting a result.
        assert_eq!(
            session.open_effects(),
            1,
            "a surfaced control/signature is an OPEN dispatched effect awaiting the host's answer"
        );
        let dispatched_family = session
            .log()
            .iter()
            .find_map(|e| match &e.body {
                EventBody::Dispatched { family, .. } => Some(family.clone()),
                _ => None,
            })
            .expect("a Dispatched frame was recorded for the fold-back control");
        assert_eq!(
            dispatched_family.as_ref(),
            crate::effect::effect_ct::SIGNATURE,
            "the dispatch records control/signature (so recovery classifies it, not the emit placeholder)"
        );
        // No AuthzDenied — control is exempt even under deny_all (a routed effect would be denied).
        assert!(!session
            .log()
            .iter()
            .any(|e| matches!(e.body, EventBody::AuthzDenied { .. })));

        // The HOST reflects the target + settles the query with the descriptor bytes → the guest resumes.
        let descriptor = b"(component-signature (export (name run)))".to_vec();
        let settled = session
            .settle_control_result(
                sig_id,
                EffectOutcome::Ok(Some(crate::effect::Payload::Inline(
                    descriptor.clone().into(),
                ))),
                &SignatureQueryReducer,
                &Authorizer::deny_all(),
                &mut exec,
            )
            .await;
        assert!(settled, "settling an open fold-back control returns true");
        // The continuation resumed: the reducer folded the EffectResult + wrote the descriptor to KV.
        assert_eq!(
            session.kv().get(b"sig/descriptor").map(|b| b.to_vec()),
            Some(descriptor),
            "settle_control_result folds the descriptor back → the emitting reducer resumes with it"
        );
        // The effect is now SETTLED (removed from open) — the continuation resumed exactly once.
        assert_eq!(
            session.open_effects(),
            0,
            "the settled query is no longer open"
        );

        // At-most-once: a DUPLICATE settle (a late/retried host answer) is a no-op — no second EffectResult,
        // the continuation can't resume twice.
        let dup = session
            .settle_control_result(
                sig_id,
                EffectOutcome::Ok(Some(crate::effect::Payload::Inline(
                    b"other".to_vec().into(),
                ))),
                &SignatureQueryReducer,
                &Authorizer::deny_all(),
                &mut exec,
            )
            .await;
        assert!(
            !dup,
            "a duplicate settle of an already-settled id is a no-op"
        );
        let result_count = session
            .log()
            .iter()
            .filter(|e| matches!(e.body, EventBody::EffectResult { .. }))
            .count();
        assert_eq!(
            result_count, 1,
            "exactly one EffectResult — the dup settle appended nothing"
        );

        // Settling an id that was never dispatched is likewise a no-op (nothing open to resume).
        let never = session
            .settle_control_result(
                EffectId(9999),
                EffectOutcome::Ok(None),
                &SignatureQueryReducer,
                &Authorizer::deny_all(),
                &mut exec,
            )
            .await;
        assert!(!never, "settling an unknown/never-dispatched id is a no-op");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn settle_control_result_err_path_resumes_the_guest_with_a_failure() {
        // The host couldn't reflect the target (bad bytes, missing blob) → it settles with an Err. The
        // reducer's continuation resumes on the err arm (writes sig/error), same as a failed routed effect —
        // never stuck. Proves the fold-back seam carries a failure as cleanly as a success.
        let mut exec = RecordingExecutor::new();
        let mut session = Session::genesis(Hash::of(b"sig-err-v1"), Hash::of(b"nonce"));
        let control = session
            .deliver_control(
                inbound(),
                None,
                &SignatureQueryReducer,
                &Authorizer::deny_all(),
                &mut exec,
            )
            .await
            .expect("deliver");
        let sig_id = control[0].id;
        let settled = session
            .settle_control_result(
                sig_id,
                EffectOutcome::err("not a valid component".to_string()),
                &SignatureQueryReducer,
                &Authorizer::deny_all(),
                &mut exec,
            )
            .await;
        assert!(settled);
        assert_eq!(
            session.kv().get(b"sig/error").map(|b| b.to_vec()),
            Some(b"query-failed".to_vec()),
            "an Err settle resumes the guest on the err arm — the continuation folds cleanly, not stuck"
        );
        assert_eq!(session.open_effects(), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn control_summary_still_has_no_dispatched_frame_and_is_not_settleable() {
        // Guard the SELECTIVE dispatch: only a fold-back control (signature) gets a Dispatched frame; summary
        // stays fire-and-forget (NO frame, never OPEN). A regression that dispatched summary too would leave
        // it hanging as a never-settled open effect. So after surfacing a summary, open_effect_count is 0 and
        // there is no Dispatched frame — and settling its (non-open) id is a no-op.
        let mut exec = RecordingExecutor::new();
        let mut session = Session::genesis(Hash::of(b"sum-nodispatch-v1"), Hash::of(b"nonce"));
        let control = session
            .deliver_control(
                inbound(),
                None,
                &SummaryEmitReducer,
                &Authorizer::deny_all(),
                &mut exec,
            )
            .await
            .expect("deliver");
        assert_eq!(control.len(), 1);
        assert_eq!(
            control[0].request.content_type.family.as_ref(),
            crate::effect::effect_ct::SUMMARY
        );
        assert_eq!(
            session.open_effects(),
            0,
            "control/summary is fire-and-forget — no Dispatched frame, never an open effect"
        );
        assert!(
            !session
                .log()
                .iter()
                .any(|e| matches!(e.body, EventBody::Dispatched { .. })),
            "no Dispatched frame for a fire-and-forget summary (only fold-back controls dispatch)"
        );
        // Settling the summary's id is a no-op (it was never opened) — no phantom EffectResult.
        let settled = session
            .settle_control_result(
                control[0].id,
                EffectOutcome::Ok(None),
                &SummaryEmitReducer,
                &Authorizer::deny_all(),
                &mut exec,
            )
            .await;
        assert!(!settled, "a fire-and-forget summary id is not settleable");
    }

    // A reducer that, on an inbound message, emits a `control/capabilities` QUERY (no payload) — the guest
    // asking the kernel "what can I do?". I4: the kernel answers this INLINE (project + fold), not by
    // surfacing it to the driver.
    struct CapabilitiesQueryReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for CapabilitiesQueryReducer {
        async fn fold(&self, event: &Event, _kv: &mut Kv) -> FoldOutput {
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
        let mut session = Session::genesis(Hash::of(b"caps-v1"), Hash::of(b"test-spawn-nonce"));
        let control = session
            .deliver_control(
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

        // The durable Dispatched frame records the CONTROL family (not the `Emit` placeholder kind) — so
        // crash recovery can classify this open dispatch as control/capabilities and re-answer it inline,
        // rather than misreading it as a real emit (PR #1668 review, durability fix).
        let dispatched_family = session
            .log()
            .iter()
            .find_map(|e| match &e.body {
                EventBody::Dispatched { family, .. } => Some(family.clone()),
                _ => None,
            })
            .expect("a Dispatched frame was recorded");
        assert_eq!(
            dispatched_family.as_ref(),
            crate::effect::effect_ct::CAPABILITIES,
            "the inline-capabilities dispatch records control/capabilities, not the emit placeholder"
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

    // A reducer that folds nothing on any event — the minimal case for the genesis-seed (the seed is
    // kernel-triggered; the reducer need not react, it just receives the born-knowing manifest result).
    struct InertReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for InertReducer {
        async fn fold(&self, _event: &Event, _kv: &mut Kv) -> FoldOutput {
            FoldOutput::none()
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn genesis_seed_folds_a_capabilities_manifest_so_the_guest_is_born_knowing() {
        use crate::executor::CompositeExecutor;
        // I5: right after genesis (which folds nothing), seed_capabilities folds a synthetic
        // capabilities-manifest EffectResult — the guest is born knowing, without issuing a query. Same
        // wire shape + code path as an I4b guest query. Executor serves emit; grant permits emit.
        let mut exec = CompositeExecutor::new().with_effect(
            crate::effect::effect_ct::EMIT,
            Box::new(RecordingExecutor::new()),
        );
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Emit,
            predicate: crate::effect::ResourcePredicate::Any,
        }]);
        let mut session = Session::genesis(Hash::of(b"seed-v1"), Hash::of(b"test-spawn-nonce"));
        // Precondition: a bare genesis log has exactly the Genesis event, no manifest yet.
        assert_eq!(session.log().len(), 1);

        let surfaced = session
            .seed_capabilities(&InertReducer, &authz, &mut exec)
            .await;
        // The seed answers inline — nothing surfaces to the driver, nothing routed to the executor.
        assert!(
            surfaced.is_empty(),
            "the seed is answered inline, not surfaced"
        );

        // The seed's durable Dispatched records the control family (recovery-classifiable), cause-linked
        // to genesis.
        let dispatched_family = session
            .log()
            .iter()
            .find_map(|e| match &e.body {
                EventBody::Dispatched { family, .. } => Some(family.clone()),
                _ => None,
            })
            .expect("the seed recorded a Dispatched frame");
        assert_eq!(
            dispatched_family.as_ref(),
            crate::effect::effect_ct::CAPABILITIES,
            "the seed dispatch is classifiable as control/capabilities on recovery"
        );

        // Born knowing: a capabilities-manifest EffectResult is in the log after the seed, decodable, with
        // the served+granted emit family reading granted.
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
            .expect("the seed folded a capabilities-manifest EffectResult");
        let a = cadenza_ast::codec::decode_detailed(&payload).expect("manifest decodes");
        let root = a
            .as_form(a.root, "capabilities-manifest")
            .expect("payload is a capabilities-manifest");
        let entries = a.as_form(root[1], "entries").expect("entries");
        assert_eq!(entries.len(), crate::effect::effect_ct::ALL.len());
        // emit served + granted.
        let emit_granted = entries.iter().any(|&eid| {
            let e = a.as_form(eid, "entry").expect("entry");
            a.as_str(e[0]) == Some(crate::effect::effect_ct::EMIT)
                && a.as_form(e[1], "granted").is_some()
        });
        assert!(emit_granted, "seeded manifest reads emit as granted");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn push_capabilities_changed_pushes_on_a_surface_change_and_no_ops_when_unchanged() {
        use crate::executor::CompositeExecutor;
        // I6b: after the host mutates a session's capability surface, push_capabilities_changed folds a fresh
        // manifest IFF the projection actually moved. Model the surface change by widening the executor: the
        // session starts serving only `emit`, then `http` is added → http moves Absent→Granted, so a push
        // fires; a second push with the SAME surface is the empty-delta no-op.
        let authz = Authorizer::new(vec![
            Capability {
                kind: EffectKind::Emit,
                predicate: crate::effect::ResourcePredicate::Any,
            },
            Capability {
                kind: EffectKind::Http,
                predicate: crate::effect::ResourcePredicate::Any,
            },
        ]);
        let mut session = Session::genesis(Hash::of(b"push-v1"), Hash::of(b"test-spawn-nonce"));

        // Seed the baseline manifest with an emit-only executor (http is Absent — no executor serves it).
        let mut narrow = CompositeExecutor::new().with_effect(
            crate::effect::effect_ct::EMIT,
            Box::new(RecordingExecutor::new()),
        );
        session
            .seed_capabilities(&InertReducer, &authz, &mut narrow)
            .await;
        let cap_results = |s: &Session| {
            s.log()
                .iter()
                .filter(|e| {
                    matches!(
                        &e.body,
                        EventBody::EffectResult {
                            result: EffectOutcome::Ok(Some(_)),
                            ..
                        }
                    )
                })
                .count()
        };
        assert_eq!(cap_results(&session), 1, "seed folded one manifest");

        // Surface change: now an http executor is also present. push with the WIDER surface → http moved
        // Absent→Granted, so a capabilities-changed manifest is folded (a second result appears).
        let mut wide = CompositeExecutor::new()
            .with_effect(
                crate::effect::effect_ct::EMIT,
                Box::new(RecordingExecutor::new()),
            )
            .with_effect(
                crate::effect::effect_ct::HTTP,
                Box::new(RecordingExecutor::new()),
            );
        let surfaced = session
            .push_capabilities_changed(&InertReducer, &authz, &mut wide)
            .await;
        assert!(
            surfaced.is_empty(),
            "the push is answered inline, not surfaced"
        );
        assert_eq!(
            cap_results(&session),
            2,
            "a real surface change folds a second (capabilities-changed) manifest"
        );
        // The pushed manifest reads http as granted now (served + permitted).
        let latest_payload = session
            .log()
            .iter()
            .rev()
            .find_map(|e| match &e.body {
                EventBody::EffectResult {
                    result: EffectOutcome::Ok(Some(crate::effect::Payload::Inline(b))),
                    ..
                } => Some(b.clone()),
                _ => None,
            })
            .expect("a pushed manifest");
        let a = cadenza_ast::codec::decode_detailed(&latest_payload).expect("manifest decodes");
        let root = a
            .as_form(a.root, "capabilities-manifest")
            .expect("payload is a capabilities-manifest");
        let entries = a.as_form(root[1], "entries").expect("entries");
        let http_granted = entries.iter().any(|&eid| {
            let e = a.as_form(eid, "entry").expect("entry");
            a.as_str(e[0]) == Some(crate::effect::effect_ct::HTTP)
                && a.as_form(e[1], "granted").is_some()
        });
        assert!(http_granted, "the pushed manifest reads http as granted");

        // No change: a second push with the SAME (wide) surface is the empty-delta no-op — nothing folded.
        let log_len = session.log().len();
        let noop = session
            .push_capabilities_changed(&InertReducer, &authz, &mut wide)
            .await;
        assert!(noop.is_empty());
        assert_eq!(
            session.log().len(),
            log_len,
            "an unchanged surface pushes nothing (the manifest-didn't-move gate)"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn seed_capabilities_is_idempotent_a_second_call_is_a_noop() {
        use crate::executor::CompositeExecutor;
        // The "seed once" contract is ENFORCED (PR #1687 review): a second seed call must be a no-op — no
        // duplicate manifest dispatch/result, no mis-cause-linked (tip- rather than genesis-anchored) second
        // seed. Otherwise a double-call corrupts the log's causal provenance.
        let mut exec = CompositeExecutor::new().with_effect(
            crate::effect::effect_ct::EMIT,
            Box::new(RecordingExecutor::new()),
        );
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Emit,
            predicate: crate::effect::ResourcePredicate::Any,
        }]);
        let mut session =
            Session::genesis(Hash::of(b"seed-idem-v1"), Hash::of(b"test-spawn-nonce"));

        let first = session
            .seed_capabilities(&InertReducer, &authz, &mut exec)
            .await;
        assert!(first.is_empty(), "seed answered inline");
        let after_first = session.log().len();
        // Exactly one control/capabilities dispatch after the first seed.
        let cap_dispatches = |s: &Session| {
            s.log()
                .iter()
                .filter(|e| {
                    matches!(&e.body, EventBody::Dispatched { family, .. }
                        if family.as_ref() == crate::effect::effect_ct::CAPABILITIES)
                })
                .count()
        };
        assert_eq!(
            cap_dispatches(&session),
            1,
            "one seed dispatch after first call"
        );

        // Second call: no-op — empty return, log UNCHANGED, still exactly one seed dispatch.
        let second = session
            .seed_capabilities(&InertReducer, &authz, &mut exec)
            .await;
        assert!(second.is_empty(), "a repeat seed is a no-op");
        assert_eq!(
            session.log().len(),
            after_first,
            "a repeat seed appends nothing to the log"
        );
        assert_eq!(
            cap_dispatches(&session),
            1,
            "still exactly one seed dispatch — never double-seeded"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_prior_guest_capabilities_query_does_not_suppress_the_seed() {
        use crate::executor::CompositeExecutor;
        // PR #1704 review: the seed-once guard must key on the SEED's identity (cause==genesis), NOT on
        // "any control/capabilities dispatch" — else a GUEST-issued capabilities query (same family frame,
        // but cause-linked to an Inbound) would make already_seeded_capabilities return true and SKIP the
        // real seed. Here a guest queries capabilities FIRST, then we seed: the seed must still fire.
        let mut exec = CompositeExecutor::new().with_effect(
            crate::effect::effect_ct::EMIT,
            Box::new(RecordingExecutor::new()),
        );
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Emit,
            predicate: crate::effect::ResourcePredicate::Any,
        }]);
        let mut session = Session::genesis(
            Hash::of(b"query-then-seed-v1"),
            Hash::of(b"test-spawn-nonce"),
        );

        // Guest issues a control/capabilities query (via an inbound-triggered fold) BEFORE any seed.
        session
            .deliver(
                inbound(),
                None,
                &CapabilitiesQueryReducer,
                &authz,
                &mut exec,
            )
            .await
            .expect("guest query");
        let cap_dispatches = |s: &Session| {
            s.log()
                .iter()
                .filter(|e| {
                    matches!(&e.body, EventBody::Dispatched { family, .. }
                        if family.as_ref() == crate::effect::effect_ct::CAPABILITIES)
                })
                .count()
        };
        assert_eq!(
            cap_dispatches(&session),
            1,
            "the guest query dispatched one capabilities frame"
        );

        // Now seed — the guard must NOT be fooled by the guest's frame; the seed must still fire.
        session
            .seed_capabilities(&InertReducer, &authz, &mut exec)
            .await;
        assert_eq!(
            cap_dispatches(&session),
            2,
            "the seed fires despite a prior guest capabilities query (guard keys on cause==genesis)"
        );
        // And the genesis-caused (seed) frame is present exactly once.
        let genesis_hash = session.log()[0].hash();
        let seed_frames = session
            .log()
            .iter()
            .filter(|e| {
                matches!(&e.body, EventBody::Dispatched { family, .. }
                    if family.as_ref() == crate::effect::effect_ct::CAPABILITIES)
                    && e.cause == Some(genesis_hash)
            })
            .count();
        assert_eq!(seed_frames, 1, "exactly one genesis-caused seed dispatch");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_genesis_seeded_session_replays_identically_and_leaves_no_open_seed_dispatch() {
        use crate::executor::CompositeExecutor;
        // The seed writes a Dispatched + its answering EffectResult into the durable log. Recovery must
        // reconstruct a seeded session correctly: (1) the born-knowing KV state survives replay byte-for-
        // byte, (2) the seed's dispatch is SETTLED by its own result — NOT left in the open set (else
        // recovery would re-drive the seed as a phantom in-flight effect), and (3) the seed is NOT
        // re-executed on replay (replay folds the logged result, never re-drives) — proven by the log
        // length being preserved (no second Dispatched/EffectResult pair appears).
        let mut exec = CompositeExecutor::new().with_effect(
            crate::effect::effect_ct::EMIT,
            Box::new(RecordingExecutor::new()),
        );
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Emit,
            predicate: crate::effect::ResourcePredicate::Any,
        }]);
        let mut session =
            Session::genesis(Hash::of(b"seed-replay-v1"), Hash::of(b"test-spawn-nonce"));
        session
            .seed_capabilities(&InertReducer, &authz, &mut exec)
            .await;

        // Precondition: after seeding, the seed dispatch is already settled (result folded), nothing open.
        assert_eq!(
            session.open_effects(),
            0,
            "the seed's dispatch is settled by its answer — no open in-flight obligation"
        );
        let live_root = session.snapshot().kv_root;
        let live_len = session.log().len();

        // Replay the durable log — recovery reconstructs the same session.
        let log = session.log().to_vec();
        let replayed = Session::replay(log, &InertReducer)
            .await
            .expect("a seeded log replays");

        assert_eq!(
            replayed.snapshot().kv_root,
            live_root,
            "born-knowing KV state must survive replay identically"
        );
        assert_eq!(
            replayed.log().len(),
            live_len,
            "replay folds the logged seed result — it does not re-drive/re-seed (no new events)"
        );
        assert_eq!(
            replayed.open_effects(),
            0,
            "replay reconstructs the settled seed dispatch — not a phantom open effect to re-drive"
        );
    }

    // A reducer that emits an effect whose content-type FAMILY is `timer` but whose kind is the `Emit`
    // placeholder — the register-by-string shape (a family with no matching EffectKind variant). Proves the
    // drive loop's timer-arm decision keys on the FAMILY (seq-39), not the legacy kind enum.
    struct TimerByFamilyReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for TimerByFamilyReducer {
        async fn fold(&self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    let mut request =
                        EffectRequest::new(EffectKind::Emit, "1000", None, Timeliness::Interactive);
                    request.content_type.family = crate::effect::effect_ct::TIMER.into();
                    FoldOutput::with_effects(vec![Effect {
                        request,
                        token: None,
                    }])
                }
                _ => FoldOutput::none(),
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_timer_arm_decision_keys_on_family_not_the_kind_enum() {
        // seq-39: the kernel routes by content-type family. An effect with family=timer but the Emit
        // placeholder kind must ARM A TIMER (kernel-fired deadline), NOT get routed to the executor as an
        // emit. This pins that the drive loop's dispatch decision moved off the EffectKind enum onto family.
        let mut exec = RecordingExecutor::new();
        let mut session =
            Session::genesis(Hash::of(b"timer-family-v1"), Hash::of(b"test-spawn-nonce"));
        // A grant permitting the timer family at the "1000" target (authz keys on family too).
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Timer,
            predicate: crate::effect::ResourcePredicate::Any,
        }]);
        session
            .deliver(inbound(), None, &TimerByFamilyReducer, &authz, &mut exec)
            .await
            .expect("deliver");

        // Armed a timer at 1000 — the family drove the timer path...
        assert_eq!(
            session.next_timer_deadline(),
            Some(1000),
            "family=timer arms a timer even with the Emit placeholder kind"
        );
        // ...and it was NOT routed to the executor as an emit.
        assert_eq!(
            exec.seen.len(),
            0,
            "a timer-family effect is kernel-armed, never routed to an executor"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn genesis_hash_is_the_first_event_hash_stable_across_replay_and_per_session_unique() {
        // genesis_hash() is the host's intended SessionId (operator ruling: SessionId = genesis-hash).
        // Pin the properties name-addressing-as-identity-map depends on. Drive a REAL fold so the
        // "stable across replay" assert exercises an actual log with post-genesis events (not a bare
        // genesis) — otherwise it degenerates to determinism-of-construction (github-liaison/#2362).
        let mut s = Session::genesis(Hash::of(b"reducer-A"), Hash::of(b"nonce-1"));
        s.deliver(
            inbound(),
            None,
            &NowReducer,
            &now_cap(),
            &mut StuckClock(1000),
        )
        .await
        .unwrap();
        assert!(
            s.log().len() > 1,
            "the session must carry post-genesis events so replay-stability is a NON-vacuous claim"
        );

        // (1) It IS log[0]'s Event::hash — the canonical durable head, not the reducer body hash. It stays
        // the genesis head even after folds appended later events (identity is anchored at log[0]).
        assert_eq!(
            s.genesis_hash(),
            s.log().first().expect("genesis has a head event").hash(),
            "genesis_hash must be the hash of the genesis EVENT (log[0]), not something else"
        );
        // (2) It is NOT the reducer hash — genesis_hash wraps the reducer in the genesis event framing, so
        // the two values differ (guards against a refactor that silently aliases them).
        assert_ne!(
            s.genesis_hash(),
            Hash::of(b"reducer-A"),
            "genesis_hash hashes the whole genesis event, so it must differ from the bare reducer hash"
        );
        // (3) STABLE across a REAL replay: reconstruct the session from its OWN persisted log via
        // Session::replay (folding each event back through the reducer) and assert the identity survives.
        // This is the genuine round-trip — `replayed` is built from s.log(), not a fresh genesis.
        let replayed = Session::replay(s.log().to_vec(), &NowReducer)
            .await
            .expect("replay of a well-formed log succeeds");
        assert_eq!(
            s.genesis_hash(),
            replayed.genesis_hash(),
            "genesis_hash is anchored at the frozen genesis head, so it survives a replay round-trip \
             (the durable identity a recovered session addresses by)"
        );

        // (4) PER-SESSION UNIQUE (§lifecycle I2, the whole point of spawn_nonce): two sessions over the SAME
        // reducer but with DIFFERENT spawn_nonces get DIFFERENT genesis hashes → distinct SessionIds → no
        // registry collision. This is what makes SessionId=genesis-hash sound (the operator's "as long as
        // genesis is unique" caveat — the host mints a fresh getrandom nonce per spawn).
        let sibling = Session::genesis(Hash::of(b"reducer-A"), Hash::of(b"nonce-2"));
        assert_ne!(
            s.genesis_hash(),
            sibling.genesis_hash(),
            "same reducer + DIFFERENT spawn_nonce ⇒ DIFFERENT genesis_hash (per-session uniqueness — I2)"
        );
        // The nonce is load-bearing: same reducer AND same nonce collide (so the host MUST mint unique
        // nonces — a getrandom draw per spawn; this pins that the nonce, not the reducer, is the entropy).
        let same_nonce = Session::genesis(Hash::of(b"reducer-A"), Hash::of(b"nonce-1"));
        assert_eq!(
            s.genesis_hash(),
            same_nonce.genesis_hash(),
            "same reducer + same spawn_nonce ⇒ same genesis_hash (the nonce is the sole entropy source)"
        );
        // A DIFFERENT reducer → different id too (independent of the nonce).
        let other = Session::genesis(Hash::of(b"reducer-B"), Hash::of(b"nonce-1"));
        assert_ne!(
            s.genesis_hash(),
            other.genesis_hash(),
            "different reducers must yield different genesis hashes"
        );
        // A SPAWNED child (parent=Some) differs from a root with the same reducer+nonce — parent provenance
        // is part of the hashed genesis body, so the child-id self-certifies its parent (§6/I2).
        let child = Session::genesis_spawned(
            Hash::of(b"reducer-A"),
            Hash::of(b"nonce-1"),
            Some(Hash::of(b"parent-genesis")),
        );
        assert_ne!(
            s.genesis_hash(),
            child.genesis_hash(),
            "parent provenance is in the hashed body → a spawned child's id differs from a root's"
        );
    }

    #[test]
    fn idempotency_key_distinguishes_families_sharing_a_placeholder_kind() {
        // seq-39: the idempotency key keys on the content-type FAMILY, not the EffectKind enum tag. Two
        // effects with the SAME id + target + placeholder kind (Emit) but DIFFERENT families must get
        // DISTINCT keys — else a register-by-string family would collide its dedup handle with a real emit
        // (both carry kind=Emit), and a crash-re-drive could dedup two genuinely-different effects together.
        let id = EffectId(1);
        let mut emit = EffectRequest::new(EffectKind::Emit, "t", None, Timeliness::Interactive);
        emit.content_type.family = crate::effect::effect_ct::EMIT.into();
        // A genuine NON-control register-by-string extension family (no EffectKind variant, so it carries
        // the Emit placeholder kind) — NOT a control/* family (which the drive loop routes out before the
        // emit path, per the control-plane partition). This is the real extension-vs-emit collision case.
        let mut ext = EffectRequest::new(EffectKind::Emit, "t", None, Timeliness::Interactive);
        ext.content_type.family = "custom/metrics".into();

        assert_ne!(
            idempotency_key_for(id, &emit),
            idempotency_key_for(id, &ext),
            "same id/target/placeholder-kind but different family → distinct keys"
        );
        // Same family + id + target → SAME key (re-drive stability preserved).
        let emit2 = {
            let mut r = EffectRequest::new(EffectKind::Emit, "t", None, Timeliness::Interactive);
            r.content_type.family = crate::effect::effect_ct::EMIT.into();
            r
        };
        assert_eq!(
            idempotency_key_for(id, &emit),
            idempotency_key_for(id, &emit2),
            "identical request → identical key (crash-re-drive dedup)"
        );
    }

    // An executor whose perform ALWAYS fails with a classified error — models a real host executor
    // (Bedrock/Http/Shell) returning a recoverable EffectOutcome::Err (never a panic/drop). The
    // `PERMANENT:`/`RETRYABLE:` prefix is the classification a supervisor keys retry-vs-give-up on (§6a).
    struct FailingHttpExecutor;
    #[async_trait::async_trait(?Send)]
    impl Executor for FailingHttpExecutor {
        async fn perform(&mut self, req: &EffectRequest, _key: Hash) -> EffectOutcome {
            assert_eq!(req.kind, EffectKind::Http);
            EffectOutcome::err("PERMANENT: 400 bad request".to_string())
        }
    }

    // Emits an Http effect on inbound; on its result, records the Err reason into KV so the test can see the
    // failure reached the reducer as a normal folded event (§9d anti-stuck).
    struct HttpThenRecordReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for HttpThenRecordReducer {
        async fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => FoldOutput::with_effects(vec![Effect {
                    request: EffectRequest::new(
                        EffectKind::Http,
                        "https://ok.host/x",
                        None,
                        Timeliness::Interactive,
                    ),
                    token: None,
                }]),
                EventBody::EffectResult {
                    result:
                        EffectOutcome::Err {
                            message: reason, ..
                        },
                    ..
                } => {
                    kv.put(b"last_err".to_vec(), reason.clone().into_bytes());
                    FoldOutput::none()
                }
                _ => FoldOutput::none(),
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_routed_effect_err_folds_back_to_the_reducer_and_the_session_is_not_stuck() {
        // §6a failure-taxonomy / §9d anti-stuck for the ROUTED-effect leg: an executor Err becomes a normal
        // EffectResult event the reducer FOLDS (never a wedge, panic, or silent drop), the classified reason
        // reaches the reducer intact, and the dispatched effect SETTLES (no dangling open obligation).
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Http,
            predicate: ResourcePredicate::Any,
        }]);
        let mut exec = FailingHttpExecutor;
        let mut s = Session::genesis(Hash::of(b"http-err-v1"), Hash::of(b"test-spawn-nonce"));
        s.deliver(
            EventBody::Inbound {
                content_type: ContentType {
                    family: "message".into(),
                    version: 1,
                },
                payload: Payload::Inline(b"go".to_vec().into()),
            },
            None,
            &HttpThenRecordReducer,
            &authz,
            &mut exec,
        )
        .await
        .unwrap();

        // The Err reached the reducer as a folded EffectResult (recorded verbatim, classification prefix intact).
        assert_eq!(
            s.kv().get(b"last_err").map(|v| v.to_vec()),
            Some(b"PERMANENT: 400 bad request".to_vec()),
            "the executor Err folds back to the reducer as a normal EffectResult event"
        );
        // §9d anti-stuck: the failed effect SETTLED — no dispatched-but-unsettled obligation left hanging.
        assert_eq!(
            s.open_effects(),
            0,
            "a routed-effect Err settles the dispatch — the session isn't wedged waiting on it"
        );
        // An Err EffectResult is on the log (the durable failure record a supervisor/replay sees).
        assert!(
            s.log().iter().any(|e| matches!(
                &e.body,
                EventBody::EffectResult {
                    result: EffectOutcome::Err { .. },
                    ..
                }
            )),
            "the Err outcome is a first-class log event"
        );
    }

    // ---- GAP-4 context-compaction (Option A: pure reducer policy, ZERO kernel change) --------------------
    //
    // The /compact problem for a self-hosting agent-reducer: its context is its event log + KV, so without a
    // summarize-and-compact strategy the WORKING SET (kv) grows unbounded turn over turn. Option A (concierge
    // ruling 2026-08-07, aligned with minimize-kernel-logic): the compaction is PURE REDUCER POLICY over the
    // EXISTING kv mechanism (put/delete) — the reducer folds accumulated detail entries into a single summary
    // entry and DELETES the detail keys, bounding the working set. NO kernel mechanism (no log pruning, no
    // Checkpoint event, no replay-contract change — those are B/C, deferred to an operator ruling). This
    // fold-proof pins that the pattern composes on the existing fold+kv seam, the M3/I3 precedent: prove the
    // reducer fold in-kernel with a native Reducer before any host policy drives it.
    //
    // The reducer keys off the inbound payload: b"detail:<x>" accumulates a per-turn detail entry under
    // detail/<seq>; b"compact" folds ALL detail/* into one summary/latest entry (a trivial concatenation
    // stands in for a real model summary — the SHAPE is what's proven) + deletes every detail/* key.
    struct CompactingReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for CompactingReducer {
        async fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
            if let EventBody::Inbound { payload, .. } = &event.body {
                let crate::effect::Payload::Inline(bytes) = payload else {
                    return FoldOutput::none();
                };
                let msg = bytes.as_ref();
                if let Some(detail) = msg.strip_prefix(b"detail:") {
                    // Accumulate a detail entry keyed by a monotonic index (the count of existing detail/*).
                    let n = (0u64..)
                        .take_while(|i| kv.get(format!("detail/{i}").as_bytes()).is_some())
                        .count();
                    kv.put(format!("detail/{n}").into_bytes(), detail.to_vec());
                } else if msg == b"compact" {
                    // Fold ALL detail/* into one summary + prune the detail keys (bound the working set).
                    let mut keys: Vec<Vec<u8>> = Vec::new();
                    let mut summary: Vec<u8> = Vec::new();
                    for i in 0u64.. {
                        let k = format!("detail/{i}").into_bytes();
                        match kv.get(&k) {
                            Some(v) => {
                                if !summary.is_empty() {
                                    summary.push(b'|');
                                }
                                summary.extend_from_slice(v);
                                keys.push(k);
                            }
                            None => break,
                        }
                    }
                    kv.put(b"summary/latest".to_vec(), summary);
                    for k in keys {
                        kv.delete(&k);
                    }
                }
            }
            FoldOutput::none()
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn gap4_option_a_compaction_folds_detail_into_a_summary_and_bounds_the_working_set() {
        // Option A end-to-end: accumulate detail across turns → the working set grows → a compact turn folds
        // it into one summary + prunes the detail, so kv shrinks back. Pure reducer policy over put/delete —
        // no kernel change, no log pruning (the LOG still records every turn; only the KV working set is
        // bounded, which is what drives the per-turn /compact problem).
        let mut exec = RecordingExecutor::new();
        let mut s = Session::genesis(Hash::of(b"gap4-compact-v1"), Hash::of(b"nonce"));
        let authz = Authorizer::deny_all(); // the reducer emits no effects; authz is irrelevant here
        let feed = |body: &[u8]| EventBody::Inbound {
            content_type: ContentType {
                family: "message".into(),
                version: 1,
            },
            payload: crate::effect::Payload::Inline(body.to_vec().into()),
        };

        // Three detail turns → three detail/* entries in the working set.
        for msg in [b"detail:alpha".as_slice(), b"detail:beta", b"detail:gamma"] {
            s.deliver(feed(msg), None, &CompactingReducer, &authz, &mut exec)
                .await
                .expect("deliver detail");
        }
        assert_eq!(s.kv().get(b"detail/0"), Some(&b"alpha"[..]));
        assert_eq!(s.kv().get(b"detail/2"), Some(&b"gamma"[..]));
        assert_eq!(
            s.kv().len(),
            3,
            "three detail entries accumulated in the working set"
        );

        // A compact turn: fold detail/* → summary/latest + prune the detail keys.
        s.deliver(
            feed(b"compact"),
            None,
            &CompactingReducer,
            &authz,
            &mut exec,
        )
        .await
        .expect("deliver compact");

        // The working set is now BOUNDED — one summary entry, all detail pruned.
        assert_eq!(
            s.kv().get(b"summary/latest"),
            Some(&b"alpha|beta|gamma"[..]),
            "compaction folds every detail entry into one summary"
        );
        assert_eq!(
            s.kv().get(b"detail/0"),
            None,
            "detail keys are pruned by the fold"
        );
        assert_eq!(s.kv().get(b"detail/1"), None);
        assert_eq!(s.kv().get(b"detail/2"), None);
        assert_eq!(
            s.kv().len(),
            1,
            "the working set shrank to the single summary — bounded regardless of turns folded"
        );

        // The pattern is REPLAY-DETERMINISTIC (no kernel change): a fresh replay of the same log reconstructs
        // the identical post-compaction kv (the compaction is an ordinary fold, already on the log).
        let replayed = Session::replay(s.log().to_vec(), &CompactingReducer)
            .await
            .expect("replay a compacted session");
        assert_eq!(
            replayed.kv().get(b"summary/latest"),
            Some(&b"alpha|beta|gamma"[..])
        );
        assert_eq!(replayed.kv().get(b"detail/0"), None);
        assert_eq!(
            replayed.kv().len(),
            1,
            "replay reconstructs the identical bounded working set — compaction is just a fold on the log"
        );
    }
}

// §lifecycle I1: the Terminated marker + the first-class FoldRefused guard. A session terminated by another
// session (distinct from self-Closed) gets a durable `Terminated` log tail, and the kernel refuses every
// further fold — the guard holds on ALL append/drive entry points (deliver_control, fire_due_timers,
// time_out_effect), so a terminated session's log tail STAYS the terminal marker (github-liaison #2381).
#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use crate::authz::Authorizer;
    use crate::effect::{Capability, EffectRequest, Payload, ResourcePredicate, Timeliness};
    use crate::executor::RecordingExecutor;
    use crate::reducer::{Effect, FoldOutput, Reducer};

    fn inbound() -> EventBody {
        EventBody::Inbound {
            content_type: crate::event::ContentType {
                family: "message".into(),
                version: 1,
            },
            payload: Payload::Inline(b"go".to_vec().into()),
        }
    }

    #[test]
    fn derive_genesis_hash_matches_the_registered_session_byte_for_byte() {
        // §lifecycle I3 host-reproducibility contract (v-ah-host coordination): the host PRE-COMPUTES a
        // child's SessionId via derive_genesis_hash(reducer, nonce, parent) to return it synchronously; the
        // loop then instantiates the child via genesis_spawned with the SAME triple. The pre-computed id MUST
        // equal what the registered session reports — else the provisional id the spawn returns wouldn't
        // match the session the kernel registers. Cover root (parent=None) AND spawned-child (parent=Some).
        let (reducer, nonce, parent) = (
            Hash::of(b"child-reducer"),
            Hash::of(b"fresh-spawn-nonce"),
            Hash::of(b"parent-session-id"),
        );
        // Root: derive == the constructed root's genesis_hash.
        assert_eq!(
            Session::derive_genesis_hash(reducer, nonce, None),
            Session::genesis(reducer, nonce).genesis_hash(),
            "host pre-compute must match the registered ROOT session byte-for-byte"
        );
        // Spawned child: derive(..., Some(parent)) == the constructed child's genesis_hash.
        assert_eq!(
            Session::derive_genesis_hash(reducer, nonce, Some(parent)),
            Session::genesis_spawned(reducer, nonce, Some(parent)).genesis_hash(),
            "host pre-compute must match the registered SPAWNED CHILD byte-for-byte"
        );
        // And the two derivations differ (parent provenance participates) — a sanity check that the parent
        // arg isn't silently dropped, which would collide a child's provisional id with a root's.
        assert_ne!(
            Session::derive_genesis_hash(reducer, nonce, None),
            Session::derive_genesis_hash(reducer, nonce, Some(parent)),
        );
    }

    #[test]
    fn parent_returns_the_spawning_session_for_a_child_and_none_for_a_root() {
        // §lifecycle I7 host seam: the host reads a terminated child's parent() to route a ChildExited signal
        // back to it. A ROOT session (genesis, no parent) → None; a SPAWNED child (genesis_spawned with
        // parent=Some) → exactly that parent hash. Read-only, from the Genesis provenance.
        let reducer = Hash::of(b"child-reducer");
        let nonce = Hash::of(b"spawn-nonce");
        let parent_id = Hash::of(b"parent-session-id");
        assert_eq!(
            Session::genesis(reducer, nonce).parent(),
            None,
            "a root session has no parent"
        );
        assert_eq!(
            Session::genesis_spawned(reducer, nonce, Some(parent_id)).parent(),
            Some(parent_id),
            "a spawned child's parent() is the spawning session's id"
        );
    }

    // A reducer that ARMS a timer (deadline 1000ms) on an inbound — so fire_due_timers has something due.
    struct TimerArmingReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for TimerArmingReducer {
        async fn fold(&self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    let mut request =
                        EffectRequest::new(EffectKind::Emit, "1000", None, Timeliness::Interactive);
                    request.content_type.family = crate::effect::effect_ct::TIMER.into();
                    FoldOutput::with_effects(vec![Effect {
                        request,
                        token: None,
                    }])
                }
                _ => FoldOutput::none(),
            }
        }
    }

    fn timer_cap() -> Authorizer {
        Authorizer::new(vec![Capability {
            kind: EffectKind::Timer,
            predicate: ResourcePredicate::Any,
        }])
    }

    fn terminated_marker() -> EventBody {
        EventBody::Terminated {
            by: Hash::of(b"controller-session"),
            reason: "operator kill".into(),
        }
    }

    // Once a `Terminated` marker is the log tail, the kernel REFUSES every further fold — a first-class
    // guard (KernelError::FoldRefused), checked before the append so no event is written + no reducer runs.
    #[tokio::test(flavor = "current_thread")]
    async fn a_terminated_session_refuses_every_further_fold() {
        let mut s = Session::genesis(
            Hash::of(b"lifecycle-term-v1"),
            Hash::of(b"test-spawn-nonce"),
        );
        // A normal live fold works before termination.
        s.deliver(
            inbound(),
            None,
            &TimerArmingReducer,
            &timer_cap(),
            &mut RecordingExecutor::new(),
        )
        .await
        .expect("a live session folds normally");
        assert!(!s.is_terminated(), "not terminated before the marker");

        // Install the durable terminal marker (what the host's I5 terminate executor will append).
        s.append(terminated_marker(), None).await;
        assert!(
            s.is_terminated(),
            "the Terminated tail marks the session terminated"
        );
        let len_after_marker = s.log().len();

        // The next delivery is REFUSED — not applied, not silently dropped.
        let refused = s
            .deliver(
                inbound(),
                None,
                &TimerArmingReducer,
                &timer_cap(),
                &mut RecordingExecutor::new(),
            )
            .await;
        assert!(
            matches!(refused, Err(KernelError::FoldRefused)),
            "a fold on a terminated session must return FoldRefused, got {refused:?}"
        );
        assert_eq!(
            s.log().len(),
            len_after_marker,
            "a refused fold must NOT append any event — the terminated log stays frozen"
        );
    }

    // Terminality is DURABLE: a session recovered from a log whose tail is `Terminated` is still terminated
    // and still refuses folds — the guard rebuilds from the log, not volatile in-memory state.
    #[tokio::test(flavor = "current_thread")]
    async fn a_recovered_terminated_session_stays_terminated_and_refuses_folds() {
        let mut s = Session::genesis(
            Hash::of(b"lifecycle-term-replay-v1"),
            Hash::of(b"test-spawn-nonce"),
        );
        s.deliver(
            inbound(),
            None,
            &TimerArmingReducer,
            &timer_cap(),
            &mut RecordingExecutor::new(),
        )
        .await
        .unwrap();
        s.append(terminated_marker(), None).await;

        let recovered = Session::replay(s.log().to_vec(), &TimerArmingReducer)
            .await
            .expect("replay of a terminated log succeeds");
        assert!(
            recovered.is_terminated(),
            "a recovered session whose log tail is Terminated must still be terminated"
        );

        let mut recovered = recovered;
        let refused = recovered
            .deliver(
                inbound(),
                None,
                &TimerArmingReducer,
                &timer_cap(),
                &mut RecordingExecutor::new(),
            )
            .await;
        assert!(
            matches!(refused, Err(KernelError::FoldRefused)),
            "a recovered terminated session must still refuse folds, got {refused:?}"
        );
    }

    // github-liaison #2381 (MED): the guard must hold on fire_due_timers too. is_terminated() keys on the
    // log TAIL, so a TimerFired appended after the Terminated marker would un-tail it → is_terminated() flips
    // false → the session is foldable again. A terminated session must fire NO due timers.
    #[tokio::test(flavor = "current_thread")]
    async fn a_terminated_session_fires_no_due_timers_and_keeps_its_terminal_tail() {
        let mut s = Session::genesis(
            Hash::of(b"lifecycle-term-timer-v1"),
            Hash::of(b"test-spawn-nonce"),
        );
        s.deliver(
            inbound(),
            None,
            &TimerArmingReducer,
            &timer_cap(),
            &mut RecordingExecutor::new(),
        )
        .await
        .unwrap();
        assert_eq!(s.next_timer_deadline(), Some(1000), "a timer is armed");

        s.append(terminated_marker(), None).await;
        assert!(s.is_terminated());
        let len_after_marker = s.log().len();

        let fired = s
            .fire_due_timers(
                1500,
                &TimerArmingReducer,
                &timer_cap(),
                &mut RecordingExecutor::new(),
            )
            .await;
        assert_eq!(fired, 0, "a terminated session fires no timers");
        assert_eq!(
            s.log().len(),
            len_after_marker,
            "fire_due_timers must append nothing on a terminated session"
        );
        assert!(
            s.is_terminated(),
            "the Terminated marker is STILL the log tail — the invariant holds across fire_due_timers"
        );
    }

    // github-liaison #2381: the same guard on time_out_effect — a terminated session times out nothing, so
    // a timeout EffectResult can't un-tail the Terminated marker. The armed timer gives a real OPEN
    // obligation (open_effects() > 0); on a terminated session time_out_effect must still be a total no-op
    // that leaves the terminal tail intact — the guard short-circuits before ANY append.
    #[tokio::test(flavor = "current_thread")]
    async fn a_terminated_session_times_out_no_effect_and_keeps_its_terminal_tail() {
        let mut s = Session::genesis(
            Hash::of(b"lifecycle-term-timeout-v1"),
            Hash::of(b"test-spawn-nonce"),
        );
        // Arm a timer, so there's an OPEN obligation in the session (open_effects() > 0).
        s.deliver(
            inbound(),
            None,
            &TimerArmingReducer,
            &timer_cap(),
            &mut RecordingExecutor::new(),
        )
        .await
        .unwrap();
        assert!(
            s.open_effects() > 0,
            "the armed timer is an open obligation"
        );
        let open_id = EffectId(0); // the first (only) open id

        s.append(terminated_marker(), None).await;
        let len_after_marker = s.log().len();

        // time_out_effect on the terminated session is a no-op — no EffectResult appended, tail preserved.
        let timed_out = s
            .time_out_effect(
                open_id,
                &TimerArmingReducer,
                &timer_cap(),
                &mut RecordingExecutor::new(),
            )
            .await;
        assert!(!timed_out, "a terminated session times out no effect");
        assert_eq!(
            s.log().len(),
            len_after_marker,
            "time_out_effect must append nothing on a terminated session"
        );
        assert!(
            s.is_terminated(),
            "the Terminated marker is STILL the log tail — the invariant holds across time_out_effect"
        );
    }

    // §lifecycle I1 public seam (v-ah-host I5 ask): Session::terminate is the fold-free public way to
    // install the Terminated marker (the host's lifecycle/terminate executor drives it; append is
    // crate-private). Pins: it appends the marker + returns its hash, folds NOTHING (no reducer run), the
    // session is then terminated + refuses folds, and a SECOND terminate is rejected (no double-marker).
    #[tokio::test(flavor = "current_thread")]
    async fn terminate_installs_the_marker_fold_free_and_is_idempotent_by_rejection() {
        let mut s = Session::genesis(Hash::of(b"lifecycle-terminate-v1"), Hash::of(b"nonce"));
        s.deliver(
            inbound(),
            None,
            &TimerArmingReducer,
            &timer_cap(),
            &mut RecordingExecutor::new(),
        )
        .await
        .unwrap();
        let len_before = s.log().len();
        assert!(!s.is_terminated());
        // Capture the prior tip so we can ASSERT the marker's causal edge points at it (not just claim it).
        let prior_tip = s.log().last().expect("a tip before terminate").hash();

        // terminate() appends the marker (log grows by exactly 1) + returns its hash; the reducer did NOT
        // run on it (fold-free) — the marker is the tail, cause-linked to the prior tip.
        let by = Hash::of(b"controller-session");
        let marker_hash = s
            .terminate(by, "operator kill".to_string())
            .await
            .expect("terminate on a live session succeeds");
        assert_eq!(
            s.log().len(),
            len_before + 1,
            "terminate appends exactly one event"
        );
        assert!(s.is_terminated(), "the session is now terminated");
        let tail = s.log().last().expect("tail");
        assert_eq!(
            tail.hash(),
            marker_hash,
            "terminate returns the appended marker's hash"
        );
        assert!(
            matches!(&tail.body, EventBody::Terminated { by: b, reason } if *b == by && reason == "operator kill"),
            "the tail is the Terminated marker carrying by + reason"
        );
        // ASSERT the causal edge (github-liaison #2395 review: the doc claimed cause-linking but the test
        // never checked it — test-vacuity). The marker's cause MUST be the prior tip (§5 causal DAG).
        assert_eq!(
            tail.cause,
            Some(prior_tip),
            "the Terminated marker is cause-linked to the prior tip (causal-DAG edge)"
        );

        // A terminated session refuses folds (the I1 guard) through this seam too.
        let refused = s
            .deliver(
                inbound(),
                None,
                &TimerArmingReducer,
                &timer_cap(),
                &mut RecordingExecutor::new(),
            )
            .await;
        assert!(matches!(refused, Err(KernelError::FoldRefused)));

        // A SECOND terminate is rejected — no double-marker (idempotent-by-rejection), log unchanged.
        let len_after = s.log().len();
        let second = s.terminate(by, "again".to_string()).await;
        assert!(
            matches!(second, Err(KernelError::FoldRefused)),
            "terminating an already-terminated session returns FoldRefused, got {second:?}"
        );
        assert_eq!(
            s.log().len(),
            len_after,
            "a rejected second terminate appends nothing"
        );
    }

    // §lifecycle I2 (Spawned edge): record_spawn appends a parent→child edge fold-free + spawned_children
    // reads them back in order; the edge is cause-linked + replay-stable; a terminated parent can't spawn.
    #[tokio::test(flavor = "current_thread")]
    async fn record_spawn_appends_parent_child_edges_readable_and_replay_stable() {
        let mut parent = Session::genesis(Hash::of(b"parent-reducer"), Hash::of(b"parent-nonce"));
        parent
            .deliver(
                inbound(),
                None,
                &TimerArmingReducer,
                &timer_cap(),
                &mut RecordingExecutor::new(),
            )
            .await
            .unwrap();
        assert!(
            parent.spawned_children().is_empty(),
            "no children before any spawn"
        );

        // Record two spawn edges (fold-free): each appends exactly one Spawned event + returns its hash,
        // cause-linked to the prior tip.
        let child_a = Hash::of(b"child-a-genesis");
        let child_b = Hash::of(b"child-b-genesis");
        let len_before = parent.log().len();
        let prior_tip = parent.log().last().expect("tip").hash();
        let edge_a = parent.record_spawn(child_a).await.expect("record child A");
        assert_eq!(
            parent.log().len(),
            len_before + 1,
            "record_spawn appends exactly one event"
        );
        let tail_a = parent.log().last().expect("tail");
        assert_eq!(
            tail_a.hash(),
            edge_a,
            "record_spawn returns the edge event's hash"
        );
        assert_eq!(
            tail_a.cause,
            Some(prior_tip),
            "the Spawned edge is cause-linked to the prior tip"
        );
        parent.record_spawn(child_b).await.expect("record child B");

        // spawned_children reads both back, in spawn order — the durable parent→child tree edges.
        assert_eq!(
            parent.spawned_children(),
            vec![child_a, child_b],
            "spawned_children returns the child genesis hashes in spawn order"
        );

        // Replay-stable: a session recovered from the log has the same spawn edges.
        let replayed = Session::replay(parent.log().to_vec(), &TimerArmingReducer)
            .await
            .expect("replay of a log with Spawned edges succeeds");
        assert_eq!(
            replayed.spawned_children(),
            vec![child_a, child_b],
            "spawn edges survive a replay round-trip (durable supervision tree)"
        );

        // A TERMINATED parent can't spawn — record_spawn is refused (a dead session has no live children).
        parent
            .terminate(Hash::of(b"controller"), "kill".to_string())
            .await
            .expect("terminate the parent");
        let refused = parent.record_spawn(Hash::of(b"child-c-genesis")).await;
        assert!(
            matches!(refused, Err(KernelError::FoldRefused)),
            "a terminated parent cannot record a spawn, got {refused:?}"
        );
    }
}

// §4c slice 3b: the drive-loop store partition — a `store/*` effect, once authorized, is applied to the
// attached NameStore (not routed to an executor) and its outcome folded back.
#[cfg(test)]
mod store_effect_tests {
    use super::*;
    use crate::authz::Authorizer;
    use crate::effect::{effect_ct, EffectRequest, Payload, Timeliness};
    use crate::name_store::NameStore;
    use crate::reducer::{FoldOutput, Reducer};

    fn inbound() -> EventBody {
        EventBody::Inbound {
            content_type: crate::event::ContentType {
                family: "message".into(),
                version: 1,
            },
            payload: Payload::Inline(b"go".to_vec().into()),
        }
    }

    // Permits any `store/*` effect (the authz gate the store arm goes through). A real deployment uses a
    // Capability whose family is store/* with a name-PREFIX predicate; here we prove the arm's ROUTING +
    // apply, so a blanket store-permitting authorizer isolates that from the (coordinated) grant-shape work.
    struct AllowStore;
    #[async_trait::async_trait(?Send)]
    impl Authorize for AllowStore {
        async fn authorize(&self, req: &EffectRequest) -> Result<(), String> {
            if effect_ct::is_store_family(&req.content_type.family) {
                Ok(())
            } else {
                Err("only store/* permitted".into())
            }
        }
    }

    // A reducer that: on inbound, emits `store/set system/compiler/latest → <hash>`; when that set's
    // result arrives, emits `store/resolve system/compiler/latest`; when the resolve result arrives,
    // records the resolved hash's hex in KV so the test can read it back.
    struct SetThenResolve;
    #[async_trait::async_trait(?Send)]
    impl Reducer for SetThenResolve {
        async fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    let payload = crate::event_ast::encode_name_set(
                        NameStore::COMPILER_LATEST,
                        &Hash::of(b"compiler-wasm-v1"),
                    );
                    FoldOutput::with(vec![EffectRequest::new_with_family(
                        effect_ct::STORE_SET,
                        NameStore::COMPILER_LATEST,
                        Some(Payload::Inline(payload.into())),
                        Timeliness::Interactive,
                    )])
                }
                EventBody::EffectResult {
                    result: EffectOutcome::Ok(body),
                    ..
                } => {
                    match kv.get(b"phase") {
                        None => {
                            // The set completed → now resolve.
                            kv.put(b"phase".to_vec(), b"resolving".to_vec());
                            FoldOutput::with(vec![EffectRequest::new_with_family(
                                effect_ct::STORE_RESOLVE,
                                NameStore::COMPILER_LATEST,
                                None,
                                Timeliness::Interactive,
                            )])
                        }
                        Some(_) => {
                            // The resolve completed → record the resolved hash (decoded from the payload).
                            if let Some(Payload::Inline(bytes)) = body {
                                if let Ok((_n, h)) = crate::event_ast::decode_name_set(bytes) {
                                    kv.put(b"resolved".to_vec(), h.to_hex().into_bytes());
                                }
                            }
                            FoldOutput::none()
                        }
                    }
                }
                _ => FoldOutput::none(),
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn store_set_then_resolve_round_trips_through_the_attached_name_store() {
        let mut exec = crate::executor::RecordingExecutor::new();
        let mut s = Session::genesis(Hash::of(b"store-v1"), Hash::of(b"test-spawn-nonce"));
        s.attach_name_store(NameStore::new());

        s.deliver(inbound(), None, &SetThenResolve, &AllowStore, &mut exec)
            .await
            .unwrap();
        // (AllowStore isolates the ARM here; production grantability via Capability::for_family is proven
        // in `store_effects_are_grantable_via_capability_for_family` below and in authz.rs's family-grant test.)

        // The reducer set then resolved system/compiler/latest; the resolved hash it recorded must equal
        // the hash it set — the store round-tripped THROUGH the kernel's store arm (set applied, resolve
        // read the latest). And the executor NEVER saw a store effect (it's not executor-routed).
        assert_eq!(
            s.kv().get(b"resolved"),
            Some(Hash::of(b"compiler-wasm-v1").to_hex().as_bytes())
        );
        assert!(
            exec.seen.is_empty(),
            "store/* is NOT routed to the executor"
        );
        assert_eq!(s.open_effects(), 0, "both store effects settled");
    }

    // §4c session-directory I3b: a reducer that JOINs two members to a group then RESOLVE-ALLs it, driving the
    // group store verbs (store/add, store/resolve-all) through the kernel's group arm end-to-end.
    struct JoinThenResolveAll {
        group: &'static str,
        m1: Hash,
        m2: Hash,
        origin: Hash,
    }
    #[async_trait::async_trait(?Send)]
    impl Reducer for JoinThenResolveAll {
        async fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    // Add m1 and m2 (each tagged (origin, seq)) in one turn — two store/add effects.
                    let add = |member: &Hash, seq: u64| {
                        let payload = crate::event_ast::encode_member_op(
                            self.group,
                            true,
                            member,
                            &(self.origin, seq),
                        );
                        EffectRequest::new_with_family(
                            effect_ct::STORE_ADD,
                            self.group,
                            Some(Payload::Inline(payload.into())),
                            Timeliness::Interactive,
                        )
                    };
                    FoldOutput::with(vec![add(&self.m1, 0), add(&self.m2, 1)])
                }
                EventBody::EffectResult {
                    result: EffectOutcome::Ok(body),
                    ..
                } => {
                    // Count settled effects; after BOTH adds settle, fire the resolve-all; on the resolve-all
                    // result, record the decoded membership COUNT into the KV (the count is what the assertion
                    // below checks; the member hashes themselves aren't stored).
                    let n = kv.get(b"settled").map(|v| v[0]).unwrap_or(0) + 1;
                    kv.put(b"settled".to_vec(), vec![n]);
                    if let Some(Payload::Inline(bytes)) = body {
                        // A members payload only rides the resolve-all result — decode it if present.
                        if let Ok(members) = crate::event_ast::decode_members(bytes) {
                            kv.put(b"member_count".to_vec(), vec![members.len() as u8]);
                        }
                    }
                    if n == 2 {
                        // Both adds settled → resolve-all the group.
                        return FoldOutput::with(vec![EffectRequest::new_with_family(
                            effect_ct::STORE_RESOLVE_ALL,
                            self.group,
                            None,
                            Timeliness::Interactive,
                        )]);
                    }
                    FoldOutput::none()
                }
                _ => FoldOutput::none(),
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn group_add_then_resolve_all_round_trips_through_the_kernel_group_arm() {
        let mut exec = crate::executor::RecordingExecutor::new();
        let mut s = Session::genesis(Hash::of(b"dir-v1"), Hash::of(b"test-spawn-nonce"));
        s.attach_name_store(NameStore::new());
        let reducer = JoinThenResolveAll {
            group: "session/room/lobby",
            m1: Hash::of(b"member-A"),
            m2: Hash::of(b"member-B"),
            origin: Hash::of(b"origin-session"),
        };

        s.deliver(inbound(), None, &reducer, &AllowStore, &mut exec)
            .await
            .unwrap();

        // The reducer added 2 members then resolve-all'd → the decoded membership it recorded is exactly 2
        // (the group round-tripped THROUGH the kernel's group arm: adds applied, resolve-all folded add-wins).
        assert_eq!(
            s.kv().get(b"member_count"),
            Some([2u8].as_slice()),
            "resolve-all returned both joined members"
        );
        // Ground-truth the attached store directly: its OR-set really has both members.
        assert_eq!(
            s.name_store()
                .unwrap()
                .resolve_all("session/room/lobby")
                .unwrap(),
            [Hash::of(b"member-A"), Hash::of(b"member-B")]
                .into_iter()
                .collect()
        );
        assert!(
            exec.seen.is_empty(),
            "store/* is NOT routed to the executor"
        );
        assert_eq!(s.open_effects(), 0, "all group store effects settled");
    }

    // ── §GAP-1 M3: the tool-calling AGENT LOOP is a FOLD over existing effects (NO new kernel mechanism) ──

    /// A test authorizer that permits ANY effect — isolates the loop mechanics (the real harness gates
    /// model/shell via Cedar; this proves the FOLD composes, not the authz).
    struct AllowAllAuthz;
    #[async_trait::async_trait(?Send)]
    impl Authorize for AllowAllAuthz {
        async fn authorize(&self, _req: &EffectRequest) -> Result<(), String> {
            Ok(())
        }
    }

    /// A scripted LLM+tool transport: a `model` effect returns a canned `model-response` (first a `tool_use`
    /// asking to run the `shell` tool, then — after the tool result comes back — an `end_turn` with the
    /// answer); a `shell` effect returns a canned tool output. This stands in for v-ah-host's real Bedrock
    /// Converse transport + shell executor, so the kernel-side test proves the reducer's FOLD drives the loop.
    struct ScriptedAgentExecutor {
        model_calls: u8,
    }
    #[async_trait::async_trait(?Send)]
    impl crate::executor::Executor for ScriptedAgentExecutor {
        async fn perform(&mut self, req: &EffectRequest, _key: Hash) -> EffectOutcome {
            let family = req.content_type.family.as_ref();
            if family == effect_ct::MODEL {
                self.model_calls += 1;
                let resp = if self.model_calls == 1 {
                    // First model turn → ask to run a tool.
                    crate::event_ast::ModelResponse {
                        stop_reason: "tool_use".to_string(),
                        content: vec![crate::event_ast::ContentBlock::ToolCall {
                            id: "call-1".to_string(),
                            name: "shell".to_string(),
                            input: br#"{"cmd":"cargo test"}"#.to_vec(),
                        }],
                    }
                } else {
                    // Second model turn (after the tool result folded back) → done.
                    crate::event_ast::ModelResponse {
                        stop_reason: "end_turn".to_string(),
                        content: vec![crate::event_ast::ContentBlock::Text(
                            "all green".to_string(),
                        )],
                    }
                };
                EffectOutcome::Ok(Some(Payload::Inline(
                    crate::event_ast::encode_model_response(&resp).into(),
                )))
            } else if family == effect_ct::SHELL {
                // The dispatched tool → a canned result the reducer folds back into the conversation.
                EffectOutcome::Ok(Some(Payload::Inline(
                    b"test result: 277 passed".to_vec().into(),
                )))
            } else {
                EffectOutcome::err(format!("unexpected effect family {family:?}"))
            }
        }
    }

    /// The agent-loop reducer (native `impl Reducer`, the reference shape §GAP-1 M3). The loop is a FOLD —
    /// NO new kernel mechanism, just existing effects composed:
    /// - inbound → emit a `model` effect carrying an `encode_model_request` (the task + the `shell` tool).
    /// - model EffectResult → `decode_model_response`: `tool_use` ⇒ dispatch each tool-call as its effect
    ///   (here `shell`); `end_turn` ⇒ record the answer (loop done).
    /// - shell EffectResult (a tool result) → emit the NEXT `model` effect (a real reducer appends a
    ///   ToolResult turn to its KV conversation; here we just re-emit to drive the loop) → back to model.
    struct AgentLoopReducer {
        model: &'static str,
    }
    impl AgentLoopReducer {
        fn model_effect(&self, req: &crate::event_ast::ModelRequest) -> EffectRequest {
            EffectRequest::new_with_family(
                effect_ct::MODEL,
                self.model,
                Some(Payload::Inline(
                    crate::event_ast::encode_model_request(req).into(),
                )),
                Timeliness::Interactive,
            )
        }
    }
    #[async_trait::async_trait(?Send)]
    impl Reducer for AgentLoopReducer {
        async fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    // Kick the loop: a model request with the task + the shell tool offered.
                    let req = crate::event_ast::ModelRequest {
                        model: self.model.to_string(),
                        messages: vec![crate::event_ast::ChatMessage {
                            role: "user".to_string(),
                            content: vec![crate::event_ast::ContentBlock::Text(
                                "run the tests".to_string(),
                            )],
                        }],
                        tools: vec![crate::event_ast::ToolDef {
                            name: "shell".to_string(),
                            schema: br#"{"type":"object"}"#.to_vec(),
                        }],
                        max_tokens: Some(1024),
                    };
                    FoldOutput::with(vec![self.model_effect(&req)])
                }
                EventBody::EffectResult {
                    result: EffectOutcome::Ok(Some(Payload::Inline(bytes))),
                    ..
                } => {
                    // Is this a MODEL response or a TOOL (shell) result? A model-response decodes; a shell
                    // result doesn't (it's raw tool output) — the reducer distinguishes by trying the codec.
                    if let Ok(resp) = crate::event_ast::decode_model_response(bytes) {
                        match resp.stop_reason.as_str() {
                            "tool_use" => {
                                // Dispatch each tool-call as its effect (reducer-defined tool→effect map: the
                                // `shell` tool → the `shell` family). Here: exactly one call.
                                let mut effects = Vec::new();
                                for blk in &resp.content {
                                    if let crate::event_ast::ContentBlock::ToolCall {
                                        name,
                                        input,
                                        ..
                                    } = blk
                                    {
                                        if name == "shell" {
                                            effects.push(EffectRequest::new_with_family(
                                                effect_ct::SHELL,
                                                "cargo test",
                                                Some(Payload::Inline(input.clone().into())),
                                                Timeliness::Interactive,
                                            ));
                                        }
                                    }
                                }
                                FoldOutput::with(effects)
                            }
                            _ => {
                                // end_turn (or any non-tool_use) → the loop is DONE; record the answer text.
                                let answer: String = resp
                                    .content
                                    .iter()
                                    .filter_map(|b| match b {
                                        crate::event_ast::ContentBlock::Text(t) => Some(t.as_str()),
                                        _ => None,
                                    })
                                    .collect();
                                kv.put(b"answer".to_vec(), answer.into_bytes());
                                FoldOutput::none()
                            }
                        }
                    } else {
                        // A TOOL result → append it to the conversation + re-emit the next model turn (with the
                        // ToolResult carrying the call id, so the model correlates it). This closes the loop.
                        let req = crate::event_ast::ModelRequest {
                            model: self.model.to_string(),
                            messages: vec![crate::event_ast::ChatMessage {
                                role: "tool".to_string(),
                                content: vec![crate::event_ast::ContentBlock::ToolResult {
                                    id: "call-1".to_string(),
                                    result: bytes.to_vec(),
                                }],
                            }],
                            tools: vec![],
                            max_tokens: Some(1024),
                        };
                        FoldOutput::with(vec![self.model_effect(&req)])
                    }
                }
                _ => FoldOutput::none(),
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn agent_loop_reducer_folds_model_tool_call_result_reemit_to_end_turn() {
        // §GAP-1 M3 — the KEYSTONE proof: a tool-calling agent loop is a FOLD over EXISTING effects, with NO
        // new kernel mechanism. The reducer: inbound → model → (tool_use) → shell → (result) → model →
        // (end_turn) → records the answer. Driven to quiescence by one deliver() through the scripted
        // model+shell transport. Proves M1+M2 codecs compose into the loop the self-hosting harness needs.
        let mut exec = ScriptedAgentExecutor { model_calls: 0 };
        let mut s = Session::genesis(Hash::of(b"agent-reducer"), Hash::of(b"nonce"));
        let reducer = AgentLoopReducer {
            model: "anthropic.claude",
        };

        s.deliver(inbound(), None, &reducer, &AllowAllAuthz, &mut exec)
            .await
            .unwrap();

        // The loop ran to end_turn and recorded the final answer.
        assert_eq!(
            s.kv().get(b"answer"),
            Some(b"all green".as_slice()),
            "the agent loop folded through model→tool→model→end_turn and recorded the answer"
        );
        // The scripted transport saw exactly TWO model calls (initial + post-tool) and one shell tool call.
        assert_eq!(
            exec.model_calls, 2,
            "two model turns: tool_use then end_turn"
        );
        assert_eq!(s.open_effects(), 0, "every effect in the loop settled");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn store_effects_are_grantable_via_capability_for_family() {
        // PRODUCTION grantability: the real Authorizer + Capability::for_family grants store/set + store/
        // resolve on `system/…` names (the §4c write-authority grant). Same round-trip as above but through
        // the actual grant path, not the test-only AllowStore — proves store/* is now grantable end-to-end.
        use crate::effect::{effect_ct, Capability, ResourcePredicate};
        let authz = Authorizer::new(vec![]).with_family_grants(vec![
            Capability::for_family(
                effect_ct::STORE_SET,
                ResourcePredicate::Prefix("system/".into()),
            ),
            Capability::for_family(
                effect_ct::STORE_RESOLVE,
                ResourcePredicate::Prefix("system/".into()),
            ),
        ]);
        let mut exec = crate::executor::RecordingExecutor::new();
        let mut s = Session::genesis(Hash::of(b"store-v1"), Hash::of(b"test-spawn-nonce"));
        s.attach_name_store(NameStore::new());
        s.deliver(inbound(), None, &SetThenResolve, &authz, &mut exec)
            .await
            .unwrap();
        assert_eq!(
            s.kv().get(b"resolved"),
            Some(Hash::of(b"compiler-wasm-v1").to_hex().as_bytes()),
            "store/set + store/resolve round-trip under a real Capability::for_family grant"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_publisher_session_store_is_readable_back_and_hands_the_pointer_to_a_consumer_session(
    ) {
        // The read-back seam (`Session::name_store`) end-to-end: the "agent B runs the program the resolver
        // fetched" loop the host's shared-store slice needs. Per-session stores mean A's writes are invisible
        // to B UNLESS a driver reads A's store out and seeds B's — this proves that hand-across works with
        // only the borrowing accessor + the landed export/replay primitives (no shared handle / interior mut).
        let mut exec_a = crate::executor::RecordingExecutor::new();
        let mut publisher = Session::genesis(Hash::of(b"store-v1"), Hash::of(b"test-spawn-nonce"));
        publisher.attach_name_store(NameStore::new());
        // A publishes: store/set system/compiler/latest → compiler-wasm-v1 (then resolves it, immaterial here).
        publisher
            .deliver(inbound(), None, &SetThenResolve, &AllowStore, &mut exec_a)
            .await
            .unwrap();

        // The DRIVER reads A's store BACK OUT (borrow — A is untouched, keeps its store) and exports the
        // name→hash pointers A published. This is the step the by-value attach alone could never do.
        let published = publisher
            .name_store()
            .expect("publisher has a store attached")
            .to_set_entries();
        assert!(
            published
                .iter()
                .any(|(n, h)| n == "system/compiler/latest" && *h == Hash::of(b"compiler-wasm-v1")),
            "A's published pointer is visible via the read-back accessor"
        );

        // Seed a SEPARATE consumer session B from A's exported pointers (replay_set_entries, landed), then B
        // store/resolve's the name A published — and reads back exactly the hash A set, across the session
        // boundary, with no shared store object.
        let consumer_store =
            NameStore::replay_set_entries(published.iter().map(|(n, h)| (n.as_str(), *h)))
                .expect("A's exported entries replay into a fresh store");
        let mut exec_b = crate::executor::RecordingExecutor::new();
        let mut consumer = Session::genesis(Hash::of(b"store-v1"), Hash::of(b"test-spawn-nonce"));
        consumer.attach_name_store(consumer_store);
        consumer
            .deliver(inbound(), None, &ResolveOnly, &AllowStore, &mut exec_b)
            .await
            .unwrap();
        assert_eq!(
            consumer.kv().get(b"resolved"),
            Some(Hash::of(b"compiler-wasm-v1").to_hex().as_bytes()),
            "consumer B resolved the pointer publisher A set — the hand-across loop closes"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_published_pointer_survives_a_snapshot_bytes_round_trip_into_a_recovered_session() {
        // The DURABLE hand-across: same publish→consume loop as above, but the pointer crosses the session
        // boundary through the snapshot BLOB path (snapshot_bytes → from_snapshot_bytes), not the in-memory
        // to_set_entries/replay pair. This is exactly what a durable-store backend runs — persist publisher A's
        // store to one content-addressed blob on quiescence, then restore it into a recovered/other session on
        // recover. Each primitive is unit-tested alone (snapshot round-trip in name_store.rs, read-back +
        // in-memory hand-across above); this pins them COMPOSED, the sequence the backend actually executes.
        let mut exec_a = crate::executor::RecordingExecutor::new();
        let mut publisher = Session::genesis(Hash::of(b"store-v1"), Hash::of(b"test-spawn-nonce"));
        publisher.attach_name_store(NameStore::new());
        publisher
            .deliver(inbound(), None, &SetThenResolve, &AllowStore, &mut exec_a)
            .await
            .unwrap();

        // Driver PERSISTS A's store as a single durable blob (what a backend blob.put()s on quiescence),
        // reading it out via the borrowing accessor. Then RESTORES it into a fresh store (blob.get() on
        // recover) — no shared handle, the whole store survives as opaque bytes.
        let snapshot = publisher
            .name_store()
            .expect("publisher has a store attached")
            .snapshot_bytes();
        let restored = NameStore::from_snapshot_bytes(&snapshot)
            .expect("A's snapshot blob restores into an identical store");

        // Attach the restored store to a SEPARATE recovered consumer session B; B store/resolve's the name A
        // published and reads back exactly the hash A set — the pointer survived the durable blob boundary.
        let mut exec_b = crate::executor::RecordingExecutor::new();
        let mut consumer = Session::genesis(Hash::of(b"store-v1"), Hash::of(b"test-spawn-nonce"));
        consumer.attach_name_store(restored);
        consumer
            .deliver(inbound(), None, &ResolveOnly, &AllowStore, &mut exec_b)
            .await
            .unwrap();
        assert_eq!(
            consumer.kv().get(b"resolved"),
            Some(Hash::of(b"compiler-wasm-v1").to_hex().as_bytes()),
            "B resolved A's pointer after a snapshot_bytes→from_snapshot_bytes round-trip — durable hand-across"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn store_effect_with_no_attached_store_folds_an_observable_err() {
        // §9d anti-stuck: a store effect on a session with no NameStore attached is an observable Err
        // outcome (folded), never a panic. The reducer's set gets an Err result → it does NOT advance to
        // "resolving", so `resolved` is never written and nothing is left open.
        let mut exec = crate::executor::RecordingExecutor::new();
        let mut s = Session::genesis(Hash::of(b"store-v1"), Hash::of(b"test-spawn-nonce")); // NO attach_name_store
        s.deliver(inbound(), None, &SetThenResolve, &AllowStore, &mut exec)
            .await
            .unwrap();
        assert_eq!(s.kv().get(b"resolved"), None);
        assert_eq!(
            s.open_effects(),
            0,
            "the failed store effect settled (Err), not left open"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn unauthorized_store_effect_is_denied_at_the_gate_not_applied() {
        // The store arm sits AFTER the SEC-F1 authorize gate: an authorizer that denies store/* means the
        // set never reaches the store (AuthzDenied), so a later resolve would find nothing. Here deny-all.
        let mut exec = crate::executor::RecordingExecutor::new();
        let mut s = Session::genesis(Hash::of(b"store-v1"), Hash::of(b"test-spawn-nonce"));
        s.attach_name_store(NameStore::new());
        s.deliver(
            inbound(),
            None,
            &SetThenResolve,
            &Authorizer::deny_all(),
            &mut exec,
        )
        .await
        .unwrap();
        // Denied before apply → nothing resolved, store untouched.
        assert_eq!(s.kv().get(b"resolved"), None);
    }

    // A reducer that emits a store/set whose PAYLOAD name disagrees with the effect TARGET (a spoof: the
    // authorizer gates the target, so a mismatched payload name must be refused, not silently written).
    struct MismatchedSetReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for MismatchedSetReducer {
        async fn fold(&self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            if matches!(event.body, EventBody::Inbound { .. }) {
                // Target = system/authorized, but the payload names system/EVIL.
                let payload =
                    crate::event_ast::encode_name_set("system/evil", &Hash::of(b"evil-hash"));
                FoldOutput::with(vec![EffectRequest::new_with_family(
                    effect_ct::STORE_SET,
                    "system/authorized",
                    Some(Payload::Inline(payload.into())),
                    Timeliness::Interactive,
                )])
            } else {
                FoldOutput::none()
            }
        }
    }

    // A reducer that ONLY resolves (a pure read): on inbound it emits store/resolve; on the Ok result it
    // records the resolved hash. Used to prove read authority independently of write.
    struct ResolveOnly;
    #[async_trait::async_trait(?Send)]
    impl Reducer for ResolveOnly {
        async fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    FoldOutput::with(vec![EffectRequest::new_with_family(
                        effect_ct::STORE_RESOLVE,
                        NameStore::COMPILER_LATEST,
                        None,
                        Timeliness::Interactive,
                    )])
                }
                EventBody::EffectResult {
                    result: EffectOutcome::Ok(Some(Payload::Inline(bytes))),
                    ..
                } => {
                    if let Ok((_n, h)) = crate::event_ast::decode_name_set(bytes) {
                        kv.put(b"resolved".to_vec(), h.to_hex().into_bytes());
                    }
                    FoldOutput::none()
                }
                _ => FoldOutput::none(),
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn store_set_with_payload_name_mismatching_target_is_refused() {
        // The target is what authz gated (SEC-F1); a set whose embedded payload name differs must be an
        // observable Err, never a silent write of the payload name. The reducer never advances past the
        // failed set, so `resolved` is never recorded — the observable signal that the set was refused.
        let mut exec = crate::executor::RecordingExecutor::new();
        let mut s = Session::genesis(Hash::of(b"store-v1"), Hash::of(b"test-spawn-nonce"));
        s.attach_name_store(NameStore::new());
        s.deliver(
            inbound(),
            None,
            &MismatchedSetReducer,
            &AllowStore,
            &mut exec,
        )
        .await
        .unwrap();
        // The mismatched set folded an Err EffectResult (not Ok), so MismatchedSetReducer's Ok-arm never
        // ran and nothing was written. (The set/resolve happy-path proves the Ok flow in the sibling test.)
        assert_eq!(s.kv().get(b"resolved"), None);
        assert_eq!(
            s.open_effects(),
            0,
            "the refused set settled (Err), not left open"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn allow_read_deny_write_a_resolve_only_grant_permits_resolve_but_denies_set() {
        // Operator ask (2026-08-03): store read/write are SEPARATELY authorized — a grant of store/resolve
        // (a read) must NOT imply store/set (a write). Prove BOTH halves under a resolve-ONLY grant:
        use crate::effect::{effect_ct, Capability, ResourcePredicate};
        let resolve_only = || {
            Authorizer::new(vec![]).with_family_grants(vec![Capability::for_family(
                effect_ct::STORE_RESOLVE,
                ResourcePredicate::Prefix("system/".into()),
            )])
        };

        // (a) READ is ALLOWED: pre-seed the name (bypassing authz — direct store), then a resolve-only
        // session reads it under the resolve grant. (apply_effect takes the idempotency key — #1852.)
        let mut store = NameStore::new();
        store
            .apply_effect(
                effect_ct::STORE_SET,
                NameStore::COMPILER_LATEST,
                Some(Hash::of(b"seeded")),
                Hash::of(b"seed-key"),
            )
            .unwrap();
        let mut exec = crate::executor::RecordingExecutor::new();
        let mut reader = Session::genesis(Hash::of(b"store-v1"), Hash::of(b"test-spawn-nonce"));
        reader.attach_name_store(store);
        reader
            .deliver(inbound(), None, &ResolveOnly, &resolve_only(), &mut exec)
            .await
            .unwrap();
        assert_eq!(
            reader.kv().get(b"resolved"),
            Some(Hash::of(b"seeded").to_hex().as_bytes()),
            "a store/resolve grant PERMITS the read"
        );

        // (b) WRITE is DENIED under the same resolve-only grant: SetThenResolve emits store/set first, which
        // has no grant → AuthzDenied → the reducer never advances to resolving, nothing written.
        let mut exec2 = crate::executor::RecordingExecutor::new();
        let mut writer = Session::genesis(Hash::of(b"store-v1"), Hash::of(b"test-spawn-nonce"));
        writer.attach_name_store(NameStore::new());
        writer
            .deliver(
                inbound(),
                None,
                &SetThenResolve,
                &resolve_only(),
                &mut exec2,
            )
            .await
            .unwrap();
        assert_eq!(
            writer.kv().get(b"resolved"),
            None,
            "a store/resolve-only grant must DENY store/set (allow-read-deny-write)"
        );
    }

    // ── §4c session-NAMING increment (operator "naming next") ──────────────────────────────────────────
    // A session PUBLISHES its own identity under a `session/<name>` pointer (store/set, gated by the
    // Session name-authority), and another session RESOLVES that name to get back the publisher's genesis
    // hash — which, since SessionId = genesis-hash (I2a), IS the publisher's addressable SessionId. This is
    // the "resolve a session BY NAME → get its SessionId" path: the resolved Hash is exactly what a peer
    // would Emit to (target = hash-hex), so name-addressed cross-session messaging = resolve-then-Emit-by-id
    // with NO new kernel primitive (store/resolve + Emit + genesis-hash identity compose it). This E2E pins
    // the resolution half (name → SessionId); the host wires the resolved id into the by-id EmitExecutor.

    // A reducer that publishes a fixed (name → hash) pointer via store/set on inbound. Carries the name +
    // hash as fields so the driver can point it at `session/<name>` → the publisher's own genesis_hash.
    struct PublishName {
        name: String,
        hash: Hash,
    }
    #[async_trait::async_trait(?Send)]
    impl Reducer for PublishName {
        async fn fold(&self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    let payload = crate::event_ast::encode_name_set(&self.name, &self.hash);
                    FoldOutput::with(vec![EffectRequest::new_with_family(
                        effect_ct::STORE_SET,
                        self.name.clone(),
                        Some(Payload::Inline(payload.into())),
                        Timeliness::Interactive,
                    )])
                }
                _ => FoldOutput::none(),
            }
        }
    }

    // A reducer that resolves a fixed name on inbound + records the resolved hash (the ResolveOnly pattern,
    // but parameterized by name so it can resolve `session/<name>`).
    struct ResolveName {
        name: String,
    }
    #[async_trait::async_trait(?Send)]
    impl Reducer for ResolveName {
        async fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    FoldOutput::with(vec![EffectRequest::new_with_family(
                        effect_ct::STORE_RESOLVE,
                        self.name.clone(),
                        None,
                        Timeliness::Interactive,
                    )])
                }
                EventBody::EffectResult {
                    result: EffectOutcome::Ok(Some(Payload::Inline(bytes))),
                    ..
                } => {
                    if let Ok((_n, h)) = crate::event_ast::decode_name_set(bytes) {
                        kv.put(b"resolved".to_vec(), h.to_hex().into_bytes());
                    }
                    FoldOutput::none()
                }
                _ => FoldOutput::none(),
            }
        }
    }

    // Grants store/set + store/resolve over the `session/` prefix — the Session name-authority a session
    // needs to publish/resolve `session/<name>` pointers (mirrors AllowStore but proves the prefix path).
    fn session_store_cap() -> crate::authz::Authorizer {
        use crate::effect::{Capability, ResourcePredicate};
        crate::authz::Authorizer::new(vec![]).with_family_grants(vec![
            Capability::for_family(
                effect_ct::STORE_SET,
                ResourcePredicate::Prefix("session/".into()),
            ),
            Capability::for_family(
                effect_ct::STORE_RESOLVE,
                ResourcePredicate::Prefix("session/".into()),
            ),
        ])
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_session_published_by_name_resolves_to_its_genesis_hash_sessionid() {
        // PUBLISHER B: a session that publishes its OWN identity under `session/alice`. Its SessionId IS
        // its genesis hash (I2a), so it publishes session/alice → B.genesis_hash().
        let mut exec_b = crate::executor::RecordingExecutor::new();
        let mut publisher = Session::genesis(Hash::of(b"alice-reducer"), Hash::of(b"alice-nonce"));
        publisher.attach_name_store(NameStore::new());
        let alice_id = publisher.genesis_hash(); // = B's SessionId
        let publish = PublishName {
            name: "session/alice".to_string(),
            hash: alice_id,
        };
        publisher
            .deliver(inbound(), None, &publish, &session_store_cap(), &mut exec_b)
            .await
            .unwrap();

        // The name→hash pointer B published is visible via the read-back accessor (the host exports it to
        // seed the resolver's store — the same hand-across the compiler-latest test proves).
        let published = publisher
            .name_store()
            .expect("publisher store")
            .to_set_entries();
        assert!(
            published
                .iter()
                .any(|(n, h)| n == "session/alice" && *h == alice_id),
            "B published session/alice → its own genesis_hash (SessionId)"
        );

        // RESOLVER A: a DIFFERENT session, seeded with B's published pointer, resolves `session/alice` and
        // gets back EXACTLY B's genesis_hash — i.e. B's SessionId. That resolved hash-hex is what A would
        // Emit to (target=SessionId), so name-addressing = this resolve + the landed by-id Emit path.
        let resolver_store =
            NameStore::replay_set_entries(published.iter().map(|(n, h)| (n.as_str(), *h)))
                .expect("B's entries replay into A's store");
        let mut exec_a = crate::executor::RecordingExecutor::new();
        let mut resolver =
            Session::genesis(Hash::of(b"resolver-reducer"), Hash::of(b"resolver-nonce"));
        resolver.attach_name_store(resolver_store);
        resolver
            .deliver(
                inbound(),
                None,
                &ResolveName {
                    name: "session/alice".to_string(),
                },
                &session_store_cap(),
                &mut exec_a,
            )
            .await
            .unwrap();
        assert_eq!(
            resolver.kv().get(b"resolved"),
            Some(alice_id.to_hex().into_bytes().as_slice()),
            "resolving session/alice yields B's genesis_hash = its SessionId (name → SessionId identity)"
        );
    }

    // The Session name-authority gates `session/` writes: a session WITHOUT the session/-prefix store grant
    // is DENIED a store/set on a session/<name> (can't hijack another session's name pointer).
    #[tokio::test(flavor = "current_thread")]
    async fn publishing_a_session_name_without_the_session_prefix_grant_is_denied() {
        let mut exec = crate::executor::RecordingExecutor::new();
        let mut s = Session::genesis(Hash::of(b"nobody"), Hash::of(b"nobody-nonce"));
        s.attach_name_store(NameStore::new());
        let publish = PublishName {
            name: "session/victim".to_string(),
            hash: Hash::of(b"attacker-hash"),
        };
        // A grant over a DIFFERENT prefix (system/) — not session/ — must NOT admit the session/ write.
        let wrong_cap = {
            use crate::effect::{Capability, ResourcePredicate};
            crate::authz::Authorizer::new(vec![]).with_family_grants(vec![Capability::for_family(
                effect_ct::STORE_SET,
                ResourcePredicate::Prefix("system/".into()),
            )])
        };
        s.deliver(inbound(), None, &publish, &wrong_cap, &mut exec)
            .await
            .unwrap();
        // The set was denied at the gate → the name was never published.
        assert!(
            s.name_store()
                .expect("store")
                .to_set_entries()
                .iter()
                .all(|(n, _)| n != "session/victim"),
            "a store/set on session/victim without the session/ grant must be denied, not applied"
        );
    }

    // ---- shell pipeline per-stage authz fan-out (operator security directive) --------------------------

    use crate::effect::{Capability, EffectKind, ResourcePredicate};
    use crate::event_ast::{encode_shell_pipeline, ShellPipeline, ShellStage};
    use crate::executor::RecordingExecutor;
    use crate::hash::Hash;

    fn stage(program: &str, args: &[&str]) -> ShellStage {
        ShellStage {
            program: program.to_string(),
            args: args.iter().map(|a| a.to_string()).collect(),
        }
    }

    fn shell_pipeline_effect(stages: Vec<ShellStage>) -> EffectRequest {
        // A shell effect whose PAYLOAD carries a (shell-pipeline …) — the structured multi-stage path.
        EffectRequest::new(
            EffectKind::Shell,
            "pipeline", // bare target is ignored for a payload-carrying pipeline (stages are the authz unit)
            Some(Payload::Inline(
                encode_shell_pipeline(&ShellPipeline { stages }).into(),
            )),
            Timeliness::Interactive,
        )
    }

    // An authorizer that permits ONLY the named shell programs (OneOf allow-list), denying every other.
    fn shell_allowlist(programs: &[&str]) -> Authorizer {
        Authorizer::new(vec![Capability {
            kind: EffectKind::Shell,
            predicate: ResourcePredicate::OneOf(programs.iter().map(|p| (*p).into()).collect()),
        }])
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shell_pipeline_authz_permits_only_when_every_stage_is_allowed() {
        // Each stage's PROGRAM is authorized; the whole pipeline is permitted only if EVERY stage's program
        // is allowed. `grep | sort | head` with all three allow-listed → Ok.
        let authz = shell_allowlist(&["grep", "sort", "head"]);
        let req = shell_pipeline_effect(vec![
            stage("grep", &["-e", "needle in haystack"]),
            stage("sort", &[]),
            stage("head", &["-n", "5"]),
        ]);
        assert_eq!(
            authorize_shell_pipeline(&req, &authz).await,
            Some(Ok(())),
            "a pipeline whose every stage program is allow-listed must be authorized"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shell_pipeline_authz_denies_whole_pipeline_if_any_stage_denied() {
        // Deny-all: `head` is NOT allow-listed → the whole pipeline is denied (even though grep+sort are), and
        // the denial names the offending stage index.
        let authz = shell_allowlist(&["grep", "sort"]);
        let req = shell_pipeline_effect(vec![
            stage("grep", &["x"]),
            stage("sort", &[]),
            stage("head", &["-n", "5"]),
        ]);
        let verdict = authorize_shell_pipeline(&req, &authz)
            .await
            .expect("a pipeline is recognized (Some)");
        let err = verdict.expect_err("a pipeline with a denied stage must be denied as a whole");
        assert!(
            err.contains("stage 2"),
            "the denial should identify the offending stage (2 = head), got {err:?}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shell_pipeline_authz_rejects_empty_pipeline_and_empty_program() {
        let authz = shell_allowlist(&["echo"]);
        // Empty pipeline: recognized as a pipeline (Some) but nothing to authorize → a clean Err.
        let empty = shell_pipeline_effect(vec![]);
        assert!(authorize_shell_pipeline(&empty, &authz)
            .await
            .expect("empty pipeline is still a recognized pipeline")
            .expect_err("empty pipeline is denied")
            .contains("empty pipeline"));
        // A stage with an empty program is a malformed command → Err.
        let empty_prog = shell_pipeline_effect(vec![stage("", &["x"])]);
        assert!(authorize_shell_pipeline(&empty_prog, &authz)
            .await
            .expect("a one-stage pipeline is recognized")
            .expect_err("empty program is denied")
            .contains("empty program"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shell_pipeline_authz_falls_back_to_single_target_for_non_pipeline_payloads() {
        // The DISCRIMINANT is "the payload decodes as a (shell-pipeline …)". A shell effect whose payload is
        // NOT a pipeline (an opaque tool-call input, per the M3 agent loop), a blob-ref, or no payload at all
        // → None: the caller falls back to the ordinary single-target authorize on `req.target`. This is what
        // keeps the M3 tool-calling loop (shell effect + raw JSON input payload) working unchanged.
        let authz = shell_allowlist(&["echo"]);
        // Opaque non-pipeline inline payload (like an M3 shell tool-call's raw input) → None (not a pipeline).
        let tool_call = EffectRequest::new(
            EffectKind::Shell,
            "cargo test",
            Some(Payload::Inline(b"{\"cmd\":\"cargo test\"}".to_vec().into())),
            Timeliness::Interactive,
        );
        assert_eq!(
            authorize_shell_pipeline(&tool_call, &authz).await,
            None,
            "an opaque non-pipeline payload is NOT a pipeline → fall back to single-target authz"
        );
        // A blob-ref payload → None (the drive loop has no blob store; the bare target gates instead).
        let blob_req = EffectRequest::new(
            EffectKind::Shell,
            "cargo test",
            Some(Payload::Blob(Hash::of(b"some blob"))),
            Timeliness::Interactive,
        );
        assert_eq!(authorize_shell_pipeline(&blob_req, &authz).await, None);
        // No payload (a bare-target single command) → None.
        let bare = EffectRequest::new(EffectKind::Shell, "echo ok", None, Timeliness::Interactive);
        assert_eq!(authorize_shell_pipeline(&bare, &authz).await, None);
        // A NON-shell effect (even with a pipeline-looking payload) → None (only shell effects fan out).
        let http = EffectRequest::new(
            EffectKind::Http,
            "https://x",
            Some(Payload::Inline(
                encode_shell_pipeline(&ShellPipeline {
                    stages: vec![stage("echo", &[])],
                })
                .into(),
            )),
            Timeliness::Interactive,
        );
        assert_eq!(authorize_shell_pipeline(&http, &authz).await, None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shell_pipeline_authz_gates_the_program_not_the_args() {
        // The authorized unit is the stage PROGRAM, not its args — an allow-listed program with arbitrary
        // args (even ones that look like other commands) is permitted, because args are literal data to that
        // program, never re-interpreted (the CWE-78 discipline: no shell, args can't spawn a second program).
        let authz = shell_allowlist(&["echo"]);
        let req = shell_pipeline_effect(vec![stage("echo", &["rm", "-rf", "/"])]);
        assert_eq!(
            authorize_shell_pipeline(&req, &authz).await,
            Some(Ok(())),
            "echo with scary-looking args is fine — the args are literal data to echo, not a second program"
        );
    }

    // A reducer that emits ONE shell effect with a caller-chosen target + a (shell-pipeline …) payload — to
    // drive the SEC-F1 gate's pipeline arm end-to-end (the fan-out + the target gate compose, not just the
    // unit-level authorize_shell_pipeline).
    struct ShellPipelineEmitReducer {
        target: &'static str,
        stages: Vec<ShellStage>,
    }
    #[async_trait::async_trait(?Send)]
    impl Reducer for ShellPipelineEmitReducer {
        async fn fold(&self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => FoldOutput::with(vec![EffectRequest::new(
                    EffectKind::Shell,
                    self.target,
                    Some(Payload::Inline(
                        encode_shell_pipeline(&ShellPipeline {
                            stages: self.stages.clone(),
                        })
                        .into(),
                    )),
                    Timeliness::Interactive,
                )]),
                _ => FoldOutput::with(vec![]),
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shell_pipeline_drive_gate_permits_on_stages_alone_target_is_vestigial() {
        // TARGET-GATE RELAX (co-landed with the host pipeline executor). For a pipeline payload the per-stage
        // fan-out IS the complete SEC-F1 gate; `req.target` is NOT gated because the host pipeline executor
        // (cdz-agent-host shell.rs) runs the decoded STAGES, never `req.target`, on the pipeline path — so
        // `req.target` is vestigial and gating it would only spuriously reject a pipeline whose unused target a
        // policy happens to deny. Here the authorizer allows the stage program "echo" but NOT "rm"; the effect
        // carries the DENIED target "rm" with an ALLOWED "echo" stage. Under the relax this is PERMITTED and
        // reaches the executor — the stages authorize and the target no longer gates.
        //
        // (Before the host consumer landed this was the belt-and-suspenders regression case reviewer HIGH on
        // #2596 pinned: gate target AND stages, since the host then still direct-exec'd req.target. That gate
        // was load-bearing ONLY while the host ran the target; the host now runs the stages, so it relaxes.)
        let authz = shell_allowlist(&["echo"]); // permits program "echo", denies "rm"
        let mut exec = RecordingExecutor::new();
        let mut s = Session::genesis(Hash::of(b"pipeline-target-relax-v1"), Hash::of(b"nonce"));
        s.deliver(
            inbound(),
            None,
            &ShellPipelineEmitReducer {
                target: "rm", // vestigial on the pipeline path — the host runs the stages, not this
                stages: vec![stage("echo", &["hi"])], // the ALLOWED stage program IS the gated unit
            },
            &authz,
            &mut exec,
        )
        .await
        .expect("deliver");
        // The pipeline is authorized on its stages alone → dispatched to the executor (target NOT gated).
        assert_eq!(
            exec.seen.len(),
            1,
            "a pipeline whose STAGES are all allowed reaches the executor even if its (vestigial) target isn't"
        );
        assert!(
            !s.log()
                .iter()
                .any(|e| matches!(e.body, EventBody::AuthzDenied { .. })),
            "no AuthzDenied: the per-stage fan-out is the sole gate, the vestigial target no longer rejects"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shell_pipeline_drive_gate_still_denies_when_a_stage_is_denied() {
        // The relax drops ONLY the vestigial target gate — the per-stage fan-out remains the complete SEC-F1
        // gate. A pipeline with a DENIED stage program is still denied as a whole, even if the (now-vestigial)
        // target happens to be allowed. Here "echo" (the target) is allow-listed but the stage program "rm" is
        // NOT → the effect MUST be AuthzDenied on the stage, never reaching the executor.
        let authz = shell_allowlist(&["echo"]); // permits "echo", denies the stage program "rm"
        let mut exec = RecordingExecutor::new();
        let mut s = Session::genesis(Hash::of(b"pipeline-target-relax-v2"), Hash::of(b"nonce"));
        s.deliver(
            inbound(),
            None,
            &ShellPipelineEmitReducer {
                target: "echo",                      // allowed, but vestigial
                stages: vec![stage("rm", &["-rf"])], // DENIED stage program → whole pipeline denied
            },
            &authz,
            &mut exec,
        )
        .await
        .expect("deliver");
        assert_eq!(
            exec.seen.len(),
            0,
            "a pipeline with a denied STAGE program must not reach the executor, even with an allowed target"
        );
        assert!(
            s.log()
                .iter()
                .any(|e| matches!(e.body, EventBody::AuthzDenied { .. })),
            "the denied-stage pipeline must be AuthzDenied (the fan-out is the sole, still-enforced gate)"
        );
    }

    // ---- cadenza-docs I3: reducer-owned doc-index publish→query-back FOLD-PROOF -------------------------
    //
    // Proves the doc-publish REDUCER fold composes over LANDED mechanism — blob/* (executor-routed, here a
    // STUB blob executor stands in for v-ah-host's real BlobExecutor) + store/* (KERNEL-applied to the
    // attached NameStore) — with ZERO new kernel mechanism. The M3 precedent: prove the fold in-kernel with
    // a scripted executor before the host E2E (which needs the real BlobExecutor). corpus-bugfix I3 ruling:
    // docs register at `memory/doc/<pkg>` (memory/ = the promotion authority = the writable scope for durable
    // cross-session artifacts; `doc/` alone is Unscoped→UnscopedNameUnwritable). Effect-wire settled with
    // v-ah-host: blob/put → Ok(Inline(hash.to_hex())); blob/get(hex target) → Ok(Some(Inline(bytes)))/Ok(None).

    /// A STUB content-addressed store executor: blob/put stores bytes keyed by content hash + returns the hex
    /// hash (v-ah-host's BlobExecutor convention); blob/get(hex target) returns the stored bytes (or Ok(None)
    /// if absent). Stands in for the real host BlobExecutor so the reducer FOLD is provable in-kernel.
    struct StubBlobExecutor {
        blobs: std::collections::HashMap<String, Vec<u8>>,
    }
    #[async_trait::async_trait(?Send)]
    impl Executor for StubBlobExecutor {
        async fn perform(&mut self, req: &EffectRequest, _key: Hash) -> EffectOutcome {
            let family = req.content_type.family.as_ref();
            match family {
                effect_ct::BLOB_PUT => {
                    let bytes = match &req.payload {
                        Some(Payload::Inline(b)) => b.to_vec(),
                        _ => {
                            return EffectOutcome::err(
                                "blob/put: expected inline bytes".to_string(),
                            )
                        }
                    };
                    let hex = Hash::of(&bytes).to_hex();
                    self.blobs.insert(hex.clone(), bytes);
                    EffectOutcome::Ok(Some(Payload::Inline(hex.into_bytes().into())))
                }
                effect_ct::BLOB_GET => {
                    // Target = the hex hash (the handle blob/put returned + the reducer store/resolve'd).
                    match self.blobs.get(req.target.as_ref()) {
                        Some(bytes) => {
                            EffectOutcome::Ok(Some(Payload::Inline(bytes.clone().into())))
                        }
                        None => EffectOutcome::Ok(None), // absent = a normal answer, not an Err
                    }
                }
                other => {
                    EffectOutcome::err(format!("StubBlobExecutor: unexpected family {other:?}"))
                }
            }
        }
    }

    // Permits store/* AND blob/* (the two families the doc-publish fold uses). A real deployment scopes the
    // store grant to a memory/ prefix + the blob grant to a label; here a blanket permit isolates the FOLD
    // ROUTING+APPLY from the (coordinated) grant-shape work, same as AllowStore above.
    struct AllowStoreAndBlob;
    #[async_trait::async_trait(?Send)]
    impl Authorize for AllowStoreAndBlob {
        async fn authorize(&self, req: &EffectRequest) -> Result<(), String> {
            let fam = &req.content_type.family;
            if effect_ct::is_store_family(fam) || effect_ct::is_blob_family(fam) {
                Ok(())
            } else {
                Err("only store/* + blob/* permitted".into())
            }
        }
    }

    /// The doc-publish reducer (I3 reducer-owned doc-index). A FOLD over blob/* + store/* — NO new kernel
    /// mechanism. Two inbound messages drive it (content_type family distinguishes publish vs query):
    /// - publish (family "doc/publish"): payload = the doc-AST bytes → emit blob/put(doc-AST).
    /// - blob/put result (hex hash): emit store/set memory/doc/<pkg> = that hash (register the doc in the index).
    /// - query (family "doc/query"): emit store/resolve memory/doc/<pkg>.
    /// - store/resolve result (the name-set hash): emit blob/get(hex) to fetch the doc-AST back.
    /// - blob/get result (bytes): record the recovered doc-AST in KV under "recovered".
    struct DocPublishReducer {
        name: &'static str, // e.g. "memory/doc/cadenza-syntax"
    }
    #[async_trait::async_trait(?Send)]
    impl Reducer for DocPublishReducer {
        async fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound {
                    content_type,
                    payload,
                } => {
                    match content_type.family.as_ref() {
                        "doc/publish" => {
                            // Publish: put the doc-AST bytes into the CAS.
                            let doc = match payload {
                                Payload::Inline(b) => b.clone(),
                                Payload::Blob(_) => return FoldOutput::none(),
                            };
                            FoldOutput::with(vec![EffectRequest::new_with_family(
                                effect_ct::BLOB_PUT,
                                "doc", // label (the authz unit for the blob write)
                                Some(Payload::Inline(doc)),
                                Timeliness::Interactive,
                            )])
                        }
                        "doc/query" => {
                            // Query: resolve the doc name to its content hash.
                            FoldOutput::with(vec![EffectRequest::new_with_family(
                                effect_ct::STORE_RESOLVE,
                                self.name,
                                None,
                                Timeliness::Interactive,
                            )])
                        }
                        _ => FoldOutput::none(),
                    }
                }
                EventBody::EffectResult {
                    result: EffectOutcome::Ok(Some(Payload::Inline(bytes))),
                    ..
                } => {
                    // Distinguish the results by which phase we're in (recorded in KV):
                    // 1) blob/put result = the hex hash → store/set the doc name at it.
                    // 2) store/resolve result = a name-set payload (name, hash) → blob/get the hash.
                    // 3) blob/get result = the doc-AST bytes → record recovered.
                    if kv.get(b"published").is_none() {
                        // Phase 1: blob/put returned the hex hash. Register it in the doc index.
                        kv.put(b"published".to_vec(), bytes.to_vec()); // remember the hex hash
                        let hex = String::from_utf8_lossy(bytes).into_owned();
                        let hash = match Hash::from_hex(&hex) {
                            Some(h) => h,
                            None => return FoldOutput::none(),
                        };
                        let payload = crate::event_ast::encode_name_set(self.name, &hash);
                        FoldOutput::with(vec![EffectRequest::new_with_family(
                            effect_ct::STORE_SET,
                            self.name,
                            Some(Payload::Inline(payload.into())),
                            Timeliness::Interactive,
                        )])
                    } else if let Ok((_n, h)) = crate::event_ast::decode_name_set(bytes) {
                        // Phase 2: store/resolve returned the name-set (name, hash) → fetch the doc bytes.
                        FoldOutput::with(vec![EffectRequest::new_with_family(
                            effect_ct::BLOB_GET,
                            h.to_hex(), // the hex hash is the blob/get target
                            None,
                            Timeliness::Interactive,
                        )])
                    } else {
                        // Phase 3: blob/get returned the doc-AST bytes → recovered.
                        kv.put(b"recovered".to_vec(), bytes.to_vec());
                        FoldOutput::none()
                    }
                }
                _ => FoldOutput::none(),
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn doc_publish_index_round_trips_a_doc_ast_through_blob_put_store_set_resolve_blob_get() {
        // I3 fold-proof: a reducer PUBLISHES a doc-AST (blob/put → hex → store/set memory/doc/<pkg>) then a
        // later QUERY (store/resolve → blob/get) recovers the SAME doc-AST bytes — composing the landed
        // blob/* (stub executor) + store/* (kernel-applied) with zero new kernel mechanism.
        use crate::event::ContentType;
        let doc_ast = b"(doc-module (doc-item (name parse) (summary \"parse source\")))".to_vec();
        let name = "memory/doc/cadenza-syntax";

        let mut exec = StubBlobExecutor {
            blobs: std::collections::HashMap::new(),
        };
        let reducer = DocPublishReducer { name };
        let mut s = Session::genesis(Hash::of(b"doc-publish-v1"), Hash::of(b"nonce"));
        s.attach_name_store(NameStore::new());

        // PUBLISH: deliver a doc/publish inbound carrying the doc-AST bytes.
        let publish = EventBody::Inbound {
            content_type: ContentType {
                family: "doc/publish".into(),
                version: 1,
            },
            payload: Payload::Inline(doc_ast.clone().into()),
        };
        s.deliver(publish, None, &reducer, &AllowStoreAndBlob, &mut exec)
            .await
            .expect("publish delivers");

        // The doc index now points memory/doc/<pkg> at the content hash (registered via store/set).
        let published_hex = s
            .kv()
            .get(b"published")
            .expect("published a hex hash")
            .to_vec();
        assert_eq!(
            String::from_utf8_lossy(&published_hex),
            Hash::of(&doc_ast).to_hex(),
            "the doc-index registered the content hash of the published doc-AST"
        );

        // QUERY: deliver a doc/query inbound → resolve the name → blob/get → recover the doc-AST.
        let query = EventBody::Inbound {
            content_type: ContentType {
                family: "doc/query".into(),
                version: 1,
            },
            payload: Payload::Inline(b"".to_vec().into()),
        };
        s.deliver(query, None, &reducer, &AllowStoreAndBlob, &mut exec)
            .await
            .expect("query delivers");

        // The recovered doc-AST bytes are byte-identical to what was published (round-trip through the index).
        let recovered = s
            .kv()
            .get(b"recovered")
            .expect("recovered the doc-AST")
            .to_vec();
        assert_eq!(
            recovered, doc_ast,
            "querying the doc index recovered the exact published doc-AST (blob/put→store/set→resolve→blob/get)"
        );
    }
}
