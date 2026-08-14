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
use crate::event::{CloseOutcome, EffectOutcome, Event, EventBody};
use crate::executor::Executor;
use crate::hash::Hash;
use crate::kv::Kv;
use crate::reducer::{Effect, Reducer};
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use tracing::{debug, instrument, warn};

/// The reserved schema-hash of the well-known TIMER family — the identity a timer obligation carries
/// (schema-hash-only). Never routed (a timer obligation's `is_timer` is the real discriminant); it exists so
/// the obligation table is uniformly schema-hash-keyed. Well-known family, so the hash is always `Some`.
fn timer_schema_hash() -> Hash {
    crate::ast_marshal::effect_family_schema_hash(crate::effect::effect_ct::TIMER)
        .expect("the well-known TIMER family always has a schema-hash")
}

/// The schema-hash of the well-known CAPABILITIES control family — the schema-hash-only replacement for the
/// old `family == effect_ct::CAPABILITIES` seed-flag discriminant (I3 recovery). Well-known family → `Some`.
fn capabilities_schema_hash() -> Hash {
    crate::ast_marshal::effect_family_schema_hash(crate::effect::effect_ct::CAPABILITIES)
        .expect("the well-known CAPABILITIES family always has a schema-hash")
}

/// The schema-hash of the well-known NOW built-in effect — the schema-hash-only replacement for the old
/// `dispatch_family_of(id) == NOW` discriminant that tells a `now` result apart on replay (so `last_now`
/// rebuilds only from `now` results). Built-in kind → always `Some`.
fn now_schema_hash() -> Hash {
    crate::ast_marshal::effect_family_schema_hash(crate::effect::effect_ct::NOW)
        .expect("the well-known NOW family always has a schema-hash")
}

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

/// One entry of the resident open-obligation table (log/state-decouple I2, design D2): everything an OPEN
/// (dispatched-but-unsettled) effect id carries that the kernel used to re-derive by scanning the log for
/// its `Dispatched`/`TimerArmed` frame. Holding it resident lets `dispatch_hash_of` / `dispatch_token_of`
/// / `dispatch_family_of` / `time_out_effect` / the `status_snapshot` in-flight scan be O(1) map lookups
/// with ZERO log access — the seam that lets I5 drop the resident log. Cheaply-clonable fields (an
/// `Arc<str>` family, `Arc<[u8]>` target, a `Hash`), so the table is cheap to snapshot for a checkpoint (I6).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OpenObligation {
    /// The resolved target the frame recorded (opaque bytes — Target=Bytes). Empty for a timer.
    pub target: std::sync::Arc<[u8]>,
    /// The dispatched effect's SCHEMA-HASH identity (schema-hash-only, slice-2) — what
    /// `dispatch_schema_hash_of` returns, copied from the `Dispatched` frame. The frame identity (the legacy
    /// `kind` enum + `family` string were dropped alongside the frame's). `None` for a register-by-string
    /// EXTENSION family (no wire hash yet — phase-1a reify emits its kind as a string; those route by
    /// `content_type.family` on the input wire, phase-3). For a TIMER obligation (opened by `TimerArmed`,
    /// not `Dispatched`) this is the reserved TIMER schema-hash; `is_timer` stays the real discriminant.
    pub schema_hash: Option<Hash>,
    /// The reducer continuation token the frame carried (§19e) — what `dispatch_token_of` returns.
    /// `None` = a token-free effect/timer.
    pub token: Option<Vec<u8>>,
    /// For a TIMER obligation, its ABSOLUTE deadline in wall-clock ms (the old `armed_timers` value);
    /// `None` for a non-timer effect. Drives `fire_due_timers` + the stall-oldest computation.
    pub deadline_ms: Option<u64>,
    /// The hash of the `Dispatched` frame that opened this obligation — what `dispatch_hash_of` returns
    /// (the §16c-S1 record-result correlation). `None` for a timer (opened by `TimerArmed`, not `Dispatched`).
    pub dispatch_hash: Option<Hash>,
    /// Is this a TIMER obligation (opened by `TimerArmed`) rather than a dispatched effect? The discriminant
    /// folding the old `armed_timers` map in: `is_timer` obligations are the armed-timer set.
    pub is_timer: bool,
}

/// The set of SETTLED effect ids (terminal outcome recorded), as a WATERMARK + sparse EXCEPTIONS
/// (log/state-decouple I4, design D3) rather than a per-id `BTreeSet` that grows unboundedly for a
/// long-lived session. Effect ids are assigned in monotonic issue order, so the CONTIGUOUS settled prefix
/// collapses into `watermark` (every `id < watermark` is settled) and only the out-of-order gaps —
/// dispatched-but-not-yet-settled ids below a higher settled id — live in `exceptions`, which stays small
/// (bounded by the concurrent open frontier, not the session lifetime). `is_settled` = `id < watermark ||
/// exceptions.contains(id)`. Correctness is identical to the old set: a late result for a below-watermark
/// id still reads as settled (dropped, timeout-cancels §16c-S4).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct SettledSet {
    /// Every id STRICTLY BELOW this is settled (the contiguous settled prefix). Advances past newly-
    /// contiguous ids on each settle.
    watermark: u64,
    /// Settled ids at or above the watermark (out-of-order settles) — pruned as the watermark advances
    /// past them. Small: bounded by the concurrent open frontier, not the session's total effect count.
    exceptions: BTreeSet<u64>,
}

impl SettledSet {
    /// Is `id` settled? `id < watermark` (in the contiguous prefix) OR a recorded out-of-order exception.
    pub fn is_settled(&self, id: u64) -> bool {
        id < self.watermark || self.exceptions.contains(&id)
    }

    /// Record `id` as settled: add it as an exception, then advance the watermark past any now-contiguous
    /// run, pruning those ids from `exceptions` (so the set stays watermark + sparse gaps). Idempotent — a
    /// re-settle of an already-settled id is a no-op.
    pub fn insert(&mut self, id: u64) {
        if self.is_settled(id) {
            return;
        }
        self.exceptions.insert(id);
        // Collapse the contiguous settled prefix into the watermark.
        while self.exceptions.remove(&self.watermark) {
            self.watermark += 1;
        }
    }

    /// The contiguous-settled-prefix watermark (every id `< watermark` is settled). For the GAP-4 checkpoint
    /// frame's durable encode + recovery reconstruction (paired with [`SettledSet::exceptions`]).
    pub fn watermark(&self) -> u64 {
        self.watermark
    }

    /// The out-of-order settled ids at/above the watermark, in canonical (ascending) order — the sparse
    /// exceptions a checkpoint frame carries alongside the watermark. Bounded by the concurrent open frontier.
    pub fn exceptions(&self) -> impl Iterator<Item = u64> + '_ {
        self.exceptions.iter().copied()
    }
}

/// A single-session kernel instance: bounded resident STATE (derived KV + head/tip + open-obligation
/// table + settled watermark + id counter) with the append-only log persisted THROUGH the attached
/// [`crate::log_store::LogSink`], NOT held resident (log/state-decouple I5). The reducer/executor/authorizer
/// are supplied per operation so the same log can be replayed under a pinned reducer (the §16c-S3 "replay
/// under the version that wrote it" discipline). The full log is read back only on recovery (via
/// [`crate::log_store::LogStore::recover`] → [`Session::replay`]); nothing in steady state scans it.
pub struct Session {
    /// The GENESIS event (seq 0), held resident (log/state-decouple I1). The genesis is immutable after
    /// construction and is what [`genesis_hash`](Self::genesis_hash)/[`reducer_hash`](Self::reducer_hash)/
    /// [`genesis_provenance`](Self::genesis_provenance) read. With the resident log Vec now DROPPED (I5),
    /// this resident copy is the sole in-memory home of the head identity — recovery re-seeds it from the
    /// durable log's first event (`log[0]`), so the `genesis == log[0]` invariant still holds.
    genesis: Event,
    /// The current TIP event (last appended), held resident (log/state-decouple I1). What
    /// [`tip_hash`](Self::tip_hash) + the next-seq/snapshot reads use — off THIS field, not `self.log.last()`.
    /// Updated on every [`append`](Self::append) and rebuilt on [`replay`](Self::replay)/recovery.
    /// `tip == log[log.len()-1]` invariant (== genesis for a fresh session).
    tip: Event,
    /// The parent→child `Spawned` edges (§6/§lifecycle I2), held resident (log/state-decouple I3): the
    /// child genesis hashes this session spawned, in spawn order. What [`spawned_children`](Self::spawned_children)
    /// returns — off THIS field, not a `self.log` scan. Pushed on every `Spawned` append; rebuilt on replay.
    spawned: Vec<Hash>,
    /// Whether this session has already been seeded its capability manifest (host-capability-discovery),
    /// held resident (log/state-decouple I3). The seed is a `CAPABILITIES` control effect cause-linked to
    /// genesis (distinct from a later reactive push cause-linked to an Inbound). Set true when that seed
    /// `Dispatched` frame appends; what [`already_seeded_capabilities`](Self::already_seeded_capabilities)
    /// returns — off THIS bit, not a `self.log` scan. Rebuilt on replay.
    seeded_capabilities: bool,
    /// The session's self-close OUTCOME if it has appended a terminal `Closed` event (its normal-completion
    /// lifecycle state), held resident (log/state-decouple I3). `None` = not closed; `Some(outcome)` = closed,
    /// carrying the reducer's chosen [`crate::event::CloseOutcome`] (Success-with-payload vs Failure-with-
    /// reason) so a driver can observe WHICH way it ended, not just THAT it ended. Distinct from `Terminated`
    /// (the tip-body check `is_terminated` serves). Set from the `Closed` event's outcome when it appends;
    /// what `is_closed`, `close_outcome`, and the `status_snapshot` `Closed` read use — off THIS field, not a
    /// `self.log` scan. Rebuilt on replay (the `Closed` apply re-captures it).
    close_outcome: Option<crate::event::CloseOutcome>,
    kv: Kv,
    /// Next effect id to assign. Monotonic within the session (§16c-S4). Derived from the log on
    /// replay so it never collides after recovery.
    next_effect_id: u64,
    /// Effect ids that have a *terminal* outcome recorded (Ok/Err/TimedOut). Used to enforce
    /// timeout-cancels: a late result for a settled id is dropped (§16c-S4). A [`SettledSet`]
    /// (log/state-decouple I4, D3): watermark + sparse exceptions, so it stays bounded by the concurrent
    /// open frontier rather than growing one entry per effect for a session's whole lifetime.
    settled: SettledSet,
    /// The resident OPEN-OBLIGATION TABLE (log/state-decouple I2, design D2): effect id → the frame data
    /// an open (dispatched-but-unsettled) effect needs, so the kernel answers `dispatch_hash_of` /
    /// `dispatch_token_of` / `dispatch_family_of` / `time_out_effect` / the `status_snapshot` in-flight
    /// scan with a MAP LOOKUP, never a log walk — the decoupling that lets a later increment (I5) drop the
    /// resident log entirely. Replaces the old `open: BTreeSet<u64>` (an id-only set that forced those
    /// accessors to scan the log for the frame) AND folds in the old `armed_timers` map (a timer is an
    /// open obligation with `is_timer` + a `deadline_ms`). Populated on `Dispatched`/`TimerArmed`; drained
    /// on `EffectResult`/`TimerFired`/`AuthzDenied`. Rebuilt from the log on replay (recovery-equivalent:
    /// `replay(full) ≡ recover(checkpoint@N + tail)` — I6's gate). `BTreeMap` for a canonical (id-ordered)
    /// iteration so the in-flight scan / delta order are replay-stable.
    open: BTreeMap<u64, OpenObligation>,
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
        // Build the genesis event ONCE — the SAME construction `derive_genesis_hash` hashes, so a host
        // pre-computing the child's SessionId and this kernel registering it can never disagree (single
        // source of truth). It seeds the resident `genesis`/`tip` (log/state-decouple I1: head/tip identity
        // lives in fields). The resident log Vec is GONE (I5) — genesis reaches durable storage when the
        // caller persists it through the attached sink (the host factory does `sink.append(&genesis)`).
        let genesis_event = Self::genesis_event(reducer, spawn_nonce, parent);
        Session {
            genesis: genesis_event.clone(),
            tip: genesis_event,
            spawned: Vec::new(),
            seeded_capabilities: false,
            close_outcome: None,
            kv: Kv::new(),
            next_effect_id: 0,
            settled: SettledSet::default(),
            open: BTreeMap::new(),
            last_now: 0,
            store: None,
            persist_error: None,
            name_store: None,
            last_manifest: None,
        }
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

    /// The number of events in this session's log — derived from the resident tip (log-decouple I1/I5),
    /// NOT `self.log.len()`. Seqs are 0-based and dense (`seq = log.len()` at append) and the log is always
    /// genesis-seeded (seq 0, never empty), so the count is `tip.seq + 1`. This is the derived replacement
    /// for the `log().len()` reads scattered across callers/tests, so they stop depending on the resident
    /// log Vec ahead of its removal (I5 step 3 makes that a pure deletion).
    pub fn event_count(&self) -> u64 {
        self.tip.seq + 1
    }

    /// The session's TIP event (its most-recent log entry), held resident (log-decouple I1). This is the
    /// derived replacement for `log().last()` — callers reading the tail's hash/body/cause read it here,
    /// off the resident field, so they stop depending on the resident log Vec ahead of its removal (I5
    /// step 3). The log is always genesis-seeded (never empty), so the tip always exists — no `Option`.
    pub fn tip(&self) -> &Event {
        &self.tip
    }

    /// The session's GENESIS event (seq 0), held resident (log-decouple I1). The derived replacement for
    /// `log().first()`/`log()[0]` — read off the resident field, not the log Vec. Its counterpart is
    /// [`Session::tip`]; together they bracket the log without touching the Vec. Used by recovery/test
    /// code that seeds a durable-log source with the genesis (the constructor puts genesis in the Vec,
    /// but a write-through sink attached later must be seeded with it, exactly as the host factory does).
    pub fn genesis_ref(&self) -> &Event {
        &self.genesis
    }

    /// The reason this session MOST-RECENTLY faulted, or `None` if its tip isn't a fault. A session
    /// whose tip event is a [`EventBody::FoldFailed`] (§17: a reducer trap / fuel-exhaustion /
    /// instantiate failure the kernel captures as a first-class log event rather than a silent stall)
    /// reads `Some(reason)`; anything else — including a `FoldFailed` the session later progressed past
    /// — reads `None`, because only the TIP is the freshest signal a "what is X doing?" query wants.
    ///
    /// This is the DERIVED accessor for the one steady-state read the host status view needs off the
    /// log's fault tip (the derived [`SessionState`] has no "errored" variant — a just-faulted, idle
    /// session reads `Quiescent`, masking the fault). It reads the resident `tip` (log/state-decouple
    /// I1), NOT `self.log.last()`, so it stands on its own once the resident log Vec is dropped (I5) —
    /// letting the host stop reaching into `log()` for this. Returns an owned `Arc<str>` (cheaply
    /// clonable, and independent of the log's lifetime) rather than a borrow of the event's reason.
    pub fn last_fault_reason(&self) -> Option<std::sync::Arc<str>> {
        match &self.tip.body {
            EventBody::FoldFailed { reason, .. } => Some(reason.as_str().into()),
            _ => None,
        }
    }

    /// The current snapshot descriptor (§4): the free per-event checkpoint.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            // The tip seq — read the resident `tip` (log/state-decouple I1), not `self.log.last()`.
            seq: self.tip.seq,
            kv_root: self.kv.root_hash(),
            reducer: self.reducer_hash(),
        }
    }

    /// Assemble the [`CheckpointDescriptor`](crate::event::CheckpointDescriptor) capturing THIS session's
    /// log-derived resident state at its current tip — GAP-4 log-prune-to-checkpoint, increment #2 ("what a
    /// checkpoint must carry"). The durable checkpoint frame wraps this so recovery can resume from
    /// `[Genesis, Checkpoint, tail]` WITHOUT the pruned prefix. Genesis stays unpruned at `events[0]` (identity
    /// / reducer / provenance read from `log[0]`), so this carries ONLY what [`Session::replay`] rebuilds by
    /// folding the prefix: the KV root, the id counter + clock high-water, the settled watermark + sparse
    /// exceptions, the open-obligation table (each entry carrying its map-key id), the spawned-children edges,
    /// the capability-seed bit, and the close outcome. A PURE read of resident state — no log access, so it is
    /// as cheap as [`Session::snapshot`] and safe on the hot path. (The prune/rewrite that USES this + the
    /// recover-from-checkpoint that consumes it are later increments; the descriptor is proven-round-trippable
    /// on its own here.)
    pub fn build_checkpoint_descriptor(&self) -> crate::event::CheckpointDescriptor {
        crate::event::CheckpointDescriptor {
            kv_root: self.kv.root_hash(),
            next_effect_id: self.next_effect_id,
            last_now: self.last_now,
            settled_watermark: self.settled.watermark(),
            settled_exceptions: self.settled.exceptions().collect(),
            open: self
                .open
                .iter()
                .map(|(&id, ob)| crate::event::CheckpointObligation {
                    id,
                    target: ob.target.clone(),
                    schema_hash: ob.schema_hash,
                    token: ob.token.clone(),
                    deadline_ms: ob.deadline_ms,
                    dispatch_hash: ob.dispatch_hash,
                    is_timer: ob.is_timer,
                })
                .collect(),
            spawned: self.spawned.clone(),
            seeded_capabilities: self.seeded_capabilities,
            close_outcome: self.close_outcome.clone(),
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
        // Read the resident `genesis` (log/state-decouple I1), not `self.log.first()`.
        match &self.genesis.body {
            EventBody::Genesis {
                spawn_nonce,
                parent,
                ..
            } => (*spawn_nonce, *parent),
            _ => panic!(
                "cdz-kernel invariant violated: session's genesis event is not Genesis \
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
        // Read the resident `genesis` (log/state-decouple I1), not `self.log.first()`.
        match &self.genesis.body {
            EventBody::Genesis { reducer, .. } => *reducer,
            _ => panic!(
                "cdz-kernel invariant violated: session's genesis event is not Genesis \
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
        // Read the resident `genesis` (log/state-decouple I1), not `self.log.first()`.
        match &self.genesis {
            e @ Event {
                body: EventBody::Genesis { .. },
                ..
            } => e.hash(),
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

    /// Whether this session has CLOSED — its log carries a terminal [`EventBody::Closed`] (a reducer's
    /// clean self-completion via [`crate::reducer::FoldOutput::close`]). A cheap field read (resident flag,
    /// log-decouple I3) so a driver can observe a session transitioning to closed right after a `deliver`
    /// fold — the hook for §6 supervision's parent routing (the host sees a child close here and delivers the
    /// child-completed signal to its parent). Distinct from a `Terminated` session (ended by another).
    pub fn is_closed(&self) -> bool {
        self.close_outcome.is_some()
    }

    /// The reducer's chosen [`crate::event::CloseOutcome`] if this session has self-closed — `Some` iff
    /// [`is_closed`](Self::is_closed), carrying whether it ended in Success (with its payload) or Failure
    /// (with its reason). `None` for a still-open (or `Terminated`) session. Lets a driver / supervisor /
    /// conformance harness observe not just THAT a session closed but HOW: a self-close signalling
    /// `Success(payload)` is distinguishable from one signalling `Failure(reason)` post-close (both otherwise
    /// collapse to `SessionState::Closed`). Resident + rebuilt on replay, like [`is_closed`](Self::is_closed).
    pub fn close_outcome(&self) -> Option<&crate::event::CloseOutcome> {
        self.close_outcome.as_ref()
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
        // In-flight effects: the resident open-obligation table (log-decouple I2) carries every open id's
        // kind + target + deadline anchor, so this reads it directly — O(open) map iteration with ZERO log
        // access (I5 step 2: the seam that lets step 3 drop the resident log Vec entirely). `open` is a
        // `BTreeMap<id,_>`, so iteration is ascending id = dispatch/issue order — the same order the old
        // log scan produced. Skip `is_timer` obligations: a timer isn't an in-flight EFFECT (it's counted
        // separately as `armed_timers`); the old scan matched only `Dispatched` frames, never `TimerArmed`.
        let mut in_flight = Vec::new();
        let mut oldest_dispatch_ms: Option<u64> = None;
        for ob in self.open.values().filter(|ob| !ob.is_timer) {
            in_flight.push(InFlight {
                schema_hash: ob.schema_hash,
                // Observability only: a lossy UTF-8 view of the opaque byte target (non-UTF-8 bytes
                // render as U+FFFD). This is a human-facing status snapshot, not an authz decision,
                // so lossy is fine here (the SEC-F1 gates use the fail-closed strict view).
                target: String::from_utf8_lossy(&ob.target).into_owned(),
            });
            // The dispatch's deadline anchor (if any) doubles as its dispatch-time reference for
            // stall detection; track the oldest so a long-outstanding effect trips Stalled.
            if let Some(d) = ob.deadline_ms {
                oldest_dispatch_ms = Some(oldest_dispatch_ms.map_or(d, |o: u64| o.min(d)));
            }
        }

        // Resident close state (log-decouple I3), not a `self.log` scan: closed iff a self-close outcome
        // is present. The outcome itself is surfaced on the snapshot below so an observer sees HOW it closed.
        let closed = self.close_outcome.is_some();
        let has_work = !self.open.is_empty();
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
            // Derived from the resident tip (log-decouple I1/I5 step 2), not `self.log` — see `event_count`.
            event_count: self.event_count(),
            last_event_kind: event_body_name(&self.tip.body),
            in_flight,
            armed_timers: self.open.values().filter(|ob| ob.is_timer).count() as u32,
            published,
            // Surface the self-close outcome (Some iff closed) so an observer distinguishes a Success
            // close from a Failure close — both otherwise collapse to SessionState::Closed above.
            close_outcome: self.close_outcome.clone(),
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
        reducer: &mut dyn Reducer,
        authz: &(impl Authorize + ?Sized),
        executor: &mut (impl Executor + ?Sized),
    ) -> Result<(), KernelError> {
        // The common delivery path DROPS the surfaced control/* effects (a live session turn doesn't
        // consume them by default). A driver that needs them — `fork_for_query`'s summary watch, or a
        // signature-querier — calls [`Session::deliver_control`] instead. Keeping this `-> Result<(), _>` is
        // the never-red bridge: the downstream `cdz-agent-host` HostedSession::deliver returns this verbatim,
        // so widening it would break its build; the control-returning variant is ADDITIVE alongside it.
        let control = self
            .deliver_control(body, cause, reducer, authz, executor)
            .await?;
        // FAIL-SAFE (reviewer LOW, latent): a FOLD-BACK control (`control/signature`) was given a Dispatched
        // frame before surfacing, so it is OPEN awaiting a host settle — but the common `deliver` DROPS the
        // ControlEffect, so nothing would ever call `settle_effect_result` and the effect would orphan
        // (open forever, the reducer's continuation never resumes, quiescence/deadline logic sees a perpetual
        // open). A signature-querier is SUPPOSED to use `deliver_control`; the common `deliver` silently
        // dropping it is a supported-looking misuse. Rather than orphan, settle each dropped fold-back control
        // with an Err so the reducer RESUMES on its err arm (the same clean not-stuck outcome a failed routed
        // effect gets) — better than a perpetual open. `control/summary` (fire-and-forget, no Dispatched
        // frame, not open) is unaffected: it's simply dropped, nothing to settle.
        for ce in &control {
            if crate::effect::effect_ct::is_fold_back_control(&ce.request.content_type.family) {
                self.settle_effect_result(
                    ce.id,
                    EffectOutcome::err(
                        "fold-back control surfaced on the drop-control deliver path — use \
                         deliver_control to consume + settle it; settling Err to avoid orphaning"
                            .to_string(),
                    ),
                    reducer,
                    authz,
                    executor,
                )
                .await;
            }
        }
        Ok(())
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
        fields(event = event_body_name(&body), log_len = self.event_count())
    )]
    pub async fn deliver_control(
        &mut self,
        body: EventBody,
        cause: Option<Hash>,
        reducer: &mut dyn Reducer,
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
        // §6 self-close is terminal too (symmetric with the is_terminated guard above): a CLOSED session (a
        // reducer's `FoldOutput::close` appended a terminal `Closed`) refuses every further fold. Without this,
        // an Inbound delivered to a closed-but-not-terminated session would fold + append PAST the `Closed`
        // event, un-tailing it (a recovered session's tip would be that Inbound, not `Closed`) and breaking
        // the terminal-tip / replay-stability invariant `FoldOutput::close` introduced. Checked BEFORE the
        // append so a closed log stays frozen exactly like a terminated one — a first-class kernel guard, not
        // a host convention (even a buggy/hostile driver, or a supervisor mid-reap, can't re-drive it).
        if self.is_closed() {
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
        matches!(Some(&self.tip.body), Some(EventBody::Terminated { .. }))
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
        // §6: a self-CLOSED session is already terminal — appending `Terminated` past the terminal `Closed`
        // would un-tail it (double-terminal, and it would CLOBBER the observable self-close `CloseOutcome`).
        // Refuse, mirroring the is_terminated guard: a session that ended itself is done, not re-terminable.
        if self.is_closed() {
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
        // §6: likewise refuse a self-CLOSED session — appending a `Spawned` edge past the terminal `Closed`
        // would un-tail it. A session that ended itself can't spawn (mirrors the is_terminated guard).
        if self.is_closed() {
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
        // Resident `spawned` edge list (log-decouple I3), not a `self.log` scan.
        self.spawned.clone()
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
        reducer: &mut dyn Reducer,
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
        // The genesis event's hash — read the resident `genesis` (log-decouple I1/I5), not `log.first()`
        // (the resident Vec is gone; genesis is held resident and is the immutable head).
        let cause = self.genesis_ref().hash();
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
        // Resident `seeded_capabilities` bit (log-decouple I3), not a `self.log` scan. Set at append when
        // the genesis-caused CAPABILITIES seed frame is written (the same cause==genesis discriminant, now
        // evaluated once at append rather than re-scanned per call).
        self.seeded_capabilities
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
        reducer: &mut dyn Reducer,
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
        // Read the resident `tip` (log-decouple I1/I5), not `log.last()` (the resident Vec is gone).
        let cause = self.tip.hash();
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
        reducer: &mut dyn Reducer,
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
        // §6: a CLOSED session likewise fires no timers (symmetric with the is_terminated guard) — a timer
        // armed by a PRIOR fold must not fire a `TimerFired` past the terminal `Closed` event (same un-tailing
        // class). Return 0. Defense-in-depth: the host reaps closed sessions, but the kernel guard doesn't
        // rely on that.
        if self.is_closed() {
            return 0;
        }
        // The armed timers are the `is_timer` obligations in the open table (log-decouple I2 folded the old
        // armed_timers map in); a fired-or-not timer carries its absolute deadline_ms.
        let mut due: Vec<(u64, u64)> = self
            .open
            .iter()
            .filter_map(|(&id, ob)| match ob.deadline_ms {
                Some(deadline) if ob.is_timer && deadline <= now_ms => Some((deadline, id)),
                _ => None,
            })
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
        // The armed timers are the `is_timer` obligations (log-decouple I2); their earliest deadline.
        self.open
            .values()
            .filter(|ob| ob.is_timer)
            .filter_map(|ob| ob.deadline_ms)
            .min()
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
        // Next seq = one past the current tip (log-decouple I5: derived from the resident tip, not the
        // dropped log Vec's len). Genesis is the initial tip at seq 0, so the first append is seq 1 —
        // identical to the old `log.len()` (which was 1 after the genesis-seeded Vec).
        let seq = self.tip.seq + 1;
        let event = Event { seq, cause, body };
        let hash = event.hash();
        // Maintain the open-obligation table + settled set as obligations are created and discharged
        // (§16c-S1/S4/S5, log-decouple I2). Done AFTER the hash is known: a `Dispatched` obligation records
        // its frame hash (for `dispatch_hash_of`/`record_result` correlation). `open` is now the resident
        // OpenObligation table (folds in the old armed-timer map), so these arms carry the full frame data.
        match &event.body {
            EventBody::Dispatched {
                id,
                schema_hash,
                target,
                token,
                deadline_ms,
                ..
            } => {
                self.open.insert(
                    id.0,
                    OpenObligation {
                        target: target.clone(),
                        schema_hash: *schema_hash,
                        token: token.clone(),
                        // Carry the frame's auto-timeout deadline anchor (log-decouple I5 step 2): the
                        // `status_snapshot` stall computation reads it, so it must be resident once the
                        // in-flight scan stops reading the log. `is_timer:false` keeps it out of
                        // `fire_due_timers`/`next_timer_deadline` (both filter on `is_timer` first).
                        deadline_ms: *deadline_ms,
                        dispatch_hash: Some(hash),
                        is_timer: false,
                    },
                );
                // Resident seed flag (log-decouple I3): the capability SEED is a CAPABILITIES control
                // effect cause-linked to GENESIS (a later reactive push is cause-linked to an Inbound, so
                // the cause==genesis discriminant distinguishes them). Set the bit here so
                // `already_seeded_capabilities` is a field read, not a log scan. Keyed on the CAPABILITIES
                // family's schema-hash (schema-hash-only — the frame no longer carries the family string).
                if *schema_hash == Some(capabilities_schema_hash())
                    && event.cause == Some(self.genesis.hash())
                {
                    self.seeded_capabilities = true;
                }
            }
            EventBody::TimerArmed {
                id,
                deadline_ms,
                token,
                ..
            } => {
                self.open.insert(
                    id.0,
                    OpenObligation {
                        target: std::sync::Arc::from(&b""[..]),
                        // A timer carries no world-effect; its identity is the reserved TIMER family's
                        // schema-hash. `is_timer` is the real discriminant (this hash is never routed).
                        schema_hash: Some(timer_schema_hash()),
                        token: token.clone(),
                        deadline_ms: Some(*deadline_ms),
                        dispatch_hash: None, // opened by TimerArmed, not Dispatched
                        is_timer: true,
                    },
                );
            }
            EventBody::EffectResult { id, .. } => {
                self.open.remove(&id.0);
                self.settled.insert(id.0);
            }
            EventBody::TimerFired { id, .. } => {
                self.open.remove(&id.0);
                self.settled.insert(id.0);
            }
            // Resident lifecycle flags (log-decouple I3): the Spawned edge list + the Closed flag, so
            // `spawned_children`/the `Closed` status read are field reads, not log scans.
            EventBody::Spawned { child_hash } => {
                self.spawned.push(*child_hash);
            }
            EventBody::Closed { outcome } => {
                self.close_outcome = Some(outcome.clone());
            }
            _ => {}
        }
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
        // The just-appended event is the new TIP (log/state-decouple I1: `tip` is resident). With the log
        // Vec dropped (I5), this is the sole in-memory record of the tip — the event itself lives only in
        // the durable sink (persisted above) and is read back on recovery.
        self.tip = event;
        hash
    }

    /// Drive one fold→authorize→dispatch→fold-result turn: fold the tip, then work the resulting effects to
    /// quiescence. Reducer folds + the executor call `.await` (so a long wasm fold cooperatively yields).
    async fn drive(
        &mut self,
        reducer: &mut dyn Reducer,
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
        reducer: &mut dyn Reducer,
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
                                target: req.target.clone(),
                                idempotency_key,
                                deadline_ms: None,
                                token,
                                schema_hash: req.schema_hash,
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
                // the continuation token, so the host can later settle it by `id` via `settle_effect_result`
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
                            target: req.target.clone(),
                            idempotency_key,
                            deadline_ms: None,
                            token: token.clone(),
                            schema_hash: req.schema_hash,
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
            // AUTHZ-EXEMPT families skip the SEC-F1 capability gate (the control path already `continue`d
            // above; this covers `effect/reply`, still reached here because it is EXECUTOR-routed, not
            // control). `effect/reply`'s `target` is an opaque reply-TOKEN (not UTF-8) that a capability
            // predicate cannot admit, and the host `ReplyExecutor` cryptographically validates the token —
            // a strictly stronger gate than capability-matching. See `effect_ct::is_authz_exempt`. Routing is
            // unchanged: an exempt effect is still dispatched to its executor (no executor → unhandled, never
            // an unchecked action), so this only drops the redundant+impossible capability check.
            let authz_result =
                if crate::effect::effect_ct::is_authz_exempt(&req.content_type.family) {
                    Ok(())
                } else {
                    match authorize_shell_pipeline(&req, authz).await {
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
                    }
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
                            target: req.target.clone(),
                            idempotency_key,
                            deadline_ms: None,
                            token,
                            schema_hash: req.schema_hash,
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
                // The timer deadline is a u64 ms encoded as text in the opaque byte target; read the
                // fail-closed UTF-8 view then parse. A non-UTF-8 / non-u64 target is a malformed timer
                // (observable AuthzDenied, resumes the continuation — §9d), never a panic.
                match req.target_str().ok().and_then(|t| t.parse::<u64>().ok()) {
                    Some(deadline_ms) => {
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
                    None => {
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
                        target: req.target.clone(),
                        idempotency_key,
                        deadline_ms: None,
                        token,
                        schema_hash: req.schema_hash,
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
            let outcome = executor.perform(id, &req, idempotency_key).await;

            // DEFERRED (userspace-effects I2): the executor forwarded this effect for asynchronous
            // fulfillment (e.g. a UserspaceEffectExecutor delegated to a registered handler session) and will
            // NOT answer now — a later `settle_effect_result(id, …)` folds the real outcome. Leave the
            // `Dispatched` frame OPEN (do NOT `record_result`): the effect stays in `open`, awaiting its
            // settle, exactly like a control/signature fold-back (which is now the degenerate case of this
            // general mechanism). Deferred is a transient signal — it never becomes an `EffectResult` on the
            // log; the eventual settle folds the real Ok/Err. The continuation resumes then.
            if matches!(outcome, EffectOutcome::Deferred) {
                continue;
            }

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
    async fn fold_tip(&mut self, reducer: &mut dyn Reducer, cause: Hash) -> Vec<(Effect, Hash)> {
        // The tip to fold — the resident `tip` (log/state-decouple I1), not `self.log.last()`.
        let tip = self.tip.clone();
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
        // §6 supervision self-close: the reducer signaled CLEAN COMPLETION — append the durable Closed{outcome}
        // (its state-apply sets `self.closed`, so `is_closed()` flips), then stop. Terminal like FoldFailed:
        // a closing fold's effects are ignored (the session is ending). This is the TRIGGER for
        // `EventBody::Closed` — a session reaches it by a reducer returning `close = Some(..)` (self-close),
        // distinct from `Terminated` (another session ending it via `lifecycle/terminate`).
        if let Some(outcome) = out.close {
            self.append(EventBody::Closed { outcome }, Some(cause))
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
        reducer: &mut dyn Reducer,
        dispatch_hash: Hash,
    ) -> Vec<(Effect, Hash)> {
        if self.settled.is_settled(id.0) {
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
        reducer: &mut dyn Reducer,
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
        // §6: a CLOSED session times out nothing either — a timeout EffectResult appended after a self-close's
        // terminal Closed would un-tail it (same class). Return false, mirroring the is_terminated guard.
        if self.is_closed() {
            return false;
        }
        // Idempotent: only an OPEN id can be timed out. Settled (or never-dispatched) → no-op, so a late
        // real result and a timeout can't both settle one id (§16c-S4 at-most-once).
        if !self.open.contains_key(&id.0) {
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

    /// Settle a DEFERRED effect (by [`EffectId`]) with its real outcome — the async-fulfillment half of the
    /// userspace-effects contract (I2), and the generalization of the former `control/signature` fold-back.
    /// When an executor returns [`EffectOutcome::Deferred`] from `perform` (it forwarded the effect for
    /// asynchronous fulfillment — a `UserspaceEffectExecutor` delegating to a registered handler session, the
    /// host reflecting a `control/signature`, etc.), the kernel leaves the `Dispatched` frame OPEN; whoever
    /// fulfills the effect off-band calls this to fold the real answer back into the EMITTING reducer's
    /// continuation. The result is a logged `EffectResult` causally linked to the `Dispatched` frame and keyed
    /// by its continuation token — identical to how any routed effect (shell/http) settles — so the guest
    /// resumes exactly where it awaited, and live-kv == replayed-kv (§9d). The reducer's continuation may emit
    /// further effects; they are driven to quiescence here.
    ///
    /// FAMILY-AGNOSTIC by design: this settles ANY open dispatched effect by its `EffectId`, regardless of
    /// family — `control/signature` (the fold-back seam this generalizes) is now just the degenerate case.
    ///
    /// Idempotent + at-most-once, exactly like [`Session::time_out_effect`] (whose shape this mirrors): a
    /// TERMINATED session settles nothing (the terminal marker must stay the log tail) → `false`; an `id` that
    /// is not OPEN — already settled by a prior call, timed out, or never dispatched — is a no-op `false`, so
    /// a late or duplicate settle can never append a second `EffectResult` for one id (a continuation resumes
    /// at most once). Returns `true` iff this call settled it. `outcome` is the real answer: `Ok(..)` on
    /// success or `EffectOutcome::err(..)`/`err_retryable(..)` on failure (the reducer folds it + resumes
    /// cleanly, same as any routed effect). Settling WITH `Deferred` is nonsensical (it is a "no result yet"
    /// signal, not a result) → treated as a no-op `false` so a caller can't leave the effect open-but-"settled".
    pub async fn settle_effect_result(
        &mut self,
        id: EffectId,
        outcome: EffectOutcome,
        reducer: &mut dyn Reducer,
        authz: &(impl Authorize + ?Sized),
        executor: &mut (impl Executor + ?Sized),
    ) -> bool {
        // A TERMINATED session settles nothing — appending an EffectResult after the terminal marker would
        // un-tail it + flip is_terminated() back to false (same guard as time_out_effect/deliver_control).
        if self.is_terminated() {
            return false;
        }
        // §6: a CLOSED session likewise settles nothing — a self-close (FoldOutput::close → terminal Closed)
        // may leave an effect in-flight (Dispatched pre-close); settling its late result here would append an
        // EffectResult PAST the Closed event, un-tailing it (same terminal-tip un-tailing class as the
        // deliver/fire_due_timers guards). Guard EVERY append path (github-liaison #2381).
        if self.is_closed() {
            return false;
        }
        // Settling WITH Deferred is a no-op: Deferred is a "not answered yet" signal, never a real outcome —
        // recording it would leave the effect BOTH removed-from-open AND without a real result. Reject it.
        if matches!(outcome, EffectOutcome::Deferred) {
            return false;
        }
        // Idempotent: only an OPEN id can be settled. Settled/never-dispatched → no-op, so a late or duplicate
        // settle and any other settler (a timeout) can't both settle one id (at-most-once, §16c-S4).
        if !self.open.contains_key(&id.0) {
            return false;
        }
        // A deferred effect was opened by a `Dispatched` frame (like a routed effect), so it HAS a dispatch
        // hash. An `id` in `open` with no `Dispatched` (an armed TIMER) is not settleable here → no-op
        // `false`, mirroring time_out_effect's timer guard (never a crash).
        let Some(dispatch_hash) = self.dispatch_hash_of(id) else {
            return false;
        };
        let more = self
            .record_result(id, outcome, reducer, dispatch_hash)
            .await;
        // The reducer's continuation (now resumed with the answer) may emit further effects — drive them to
        // quiescence, same as the routed-result and timeout paths.
        self.drive_worklist(more, reducer, authz, executor).await;
        true
    }

    /// Deprecated alias for [`Session::settle_effect_result`] — the former name from when this seam only
    /// served `control/signature` fold-back (before userspace-effects I2 generalized it to ANY deferred
    /// effect). Kept as a thin delegator so the `cdz-agent-host` sig-query call-site migrates at leisure
    /// (no cross-crate build break in the I2 landing window); it will be removed once that call-site moves.
    #[deprecated(note = "renamed to settle_effect_result (userspace-effects I2, family-agnostic)")]
    pub async fn settle_control_result(
        &mut self,
        id: EffectId,
        outcome: EffectOutcome,
        reducer: &mut dyn Reducer,
        authz: &(impl Authorize + ?Sized),
        executor: &mut (impl Executor + ?Sized),
    ) -> bool {
        self.settle_effect_result(id, outcome, reducer, authz, executor)
            .await
    }

    /// The hash of the `Dispatched` event that opened effect `id`, or `None` if `id` has no `Dispatched`
    /// event — which happens for an armed TIMER id (also in `open`, but opened by `TimerArmed`, not
    /// `Dispatched`). Callers that only mean dispatched effects (e.g. `time_out_effect`) treat `None` as
    /// "not a dispatched effect" rather than an error (PR#1016 — `open` is a mixed obligation set).
    fn dispatch_hash_of(&self, id: EffectId) -> Option<Hash> {
        // O(1) lookup in the resident open-obligation table (log-decouple I2), not a log scan. The frame
        // hash is held only while the effect is OPEN (an obligation carries its `Dispatched` frame hash);
        // this is called on the open effect at record-result time, so the obligation is present. A timer
        // obligation has no dispatch_hash (opened by `TimerArmed`), so it returns None — correct.
        self.open.get(&id.0).and_then(|ob| ob.dispatch_hash)
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
        // A store NAME is text (a `system/…`/`effect/…`/group name); the opaque byte target is read as its
        // fail-closed UTF-8 view (operator Target=Bytes ruling). A non-UTF-8 target for a store effect is a
        // malformed request → observable Err, never a panic.
        let name =
            match req.target_str() {
                Ok(n) => n,
                Err(_) => return EffectOutcome::err(
                    "store effect target is not valid UTF-8 (a store name must be a UTF-8 string)"
                        .to_string(),
                ),
            };
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
        // A group NAME is text; read the opaque byte target as its fail-closed UTF-8 view (Target=Bytes
        // ruling). Non-UTF-8 → malformed group effect, observable Err.
        let name =
            match req.target_str() {
                Ok(n) => n,
                Err(_) => return EffectOutcome::err(
                    "group effect target is not valid UTF-8 (a group name must be a UTF-8 string)"
                        .to_string(),
                ),
            };
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
        // O(1) lookup in the resident open-obligation table (log-decouple I2), not a log scan — the token
        // is held on the (non-timer) obligation while it's OPEN, which is when record-result reads it.
        self.open
            .get(&id.0)
            .filter(|ob| !ob.is_timer)
            .map(|ob| ob.token.clone())
    }

    /// The SCHEMA-HASH that dispatch `id`'s `Dispatched` frame recorded, or `None` if `id` has no
    /// `Dispatched` frame (e.g. a timer, opened by `TimerArmed`). Used on replay to tell a `now` result
    /// apart (so `last_now` rebuilds only from `now` results). Keys on the durable schema-hash identity
    /// (schema-hash-only — replaces the seq-39 family string + the legacy `kind` enum). Reads the durable
    /// frame → replay-deterministic.
    fn dispatch_schema_hash_of(&self, id: EffectId) -> Option<Hash> {
        // O(1) lookup in the resident open-obligation table (log-decouple I2), not a log scan. Called when
        // an `EffectResult` folds (live + on replay) — the matching `Dispatched` obligation is still OPEN
        // at that point (the result REMOVES it), so the schema-hash is present. A timer obligation is excluded
        // (it has no `Dispatched` frame — matches the old "None if opened by TimerArmed" behavior).
        self.open
            .get(&id.0)
            .filter(|ob| !ob.is_timer)
            .and_then(|ob| ob.schema_hash)
    }

    /// The reducer continuation token that timer `id`'s `TimerArmed` frame carried (§19e slice 2b-iii),
    /// the timer analogue of [`dispatch_token_of`]: `Some(Some(token))` = a token was armed, `Some(None)`
    /// = a token-free timer, `None` = no `TimerArmed` frame for `id`. Derived from the DURABLE arming
    /// frame so it's replay-deterministic — the same fire gets the same token live or reconstructed. This
    /// is how the token "rides the TimerFired": [`fire_due_timers`] copies it onto the fire event so a
    /// wasm reducer's fold reads it back as the guest's `resumes` without fold ever touching the log/map.
    fn timer_armed_token_of(&self, id: EffectId) -> Option<Option<Vec<u8>>> {
        // O(1) lookup in the resident open-obligation table (log-decouple I2), not a log scan. A timer's
        // token is held on its `is_timer` obligation while armed; `fire_due_timers` reads it before the
        // fire removes it. `Some(Some(tok))`/`Some(None)` = armed with/without a token; `None` = not an
        // armed timer.
        self.open
            .get(&id.0)
            .filter(|ob| ob.is_timer)
            .map(|ob| ob.token.clone())
    }

    /// Hash of the current tip (last log event) — the `cause` for effects its fold emits.
    fn tip_hash(&self) -> Hash {
        // Read the resident `tip` (log/state-decouple I1), not `self.log.last()`.
        self.tip.hash()
    }

    /// Reconstruct a session from a persisted log, folding each observable event through a [`Reducer`]
    /// (`.await`) and rebuilding the obligation sets / armed-timer table / `next_effect_id` / `last_now`
    /// high-water mark.
    ///
    /// Effects emitted during replay are IGNORED (§17 "replay re-folds with no live effect" — the results
    /// are already in the log); so replayed-kv == live-kv (PR#990 finding #1).
    pub async fn replay(
        log: Vec<Event>,
        reducer: &mut dyn Reducer,
    ) -> Result<Session, KernelError> {
        let genesis_event = match log.first() {
            Some(
                e @ Event {
                    body: EventBody::Genesis { .. },
                    ..
                },
            ) => e.clone(),
            _ => return Err(KernelError::MissingGenesis),
        };
        let mut s = Session {
            // Seed the resident genesis/tip (log/state-decouple I1) from the validated first event; `tip`
            // advances to the last event as the replay loop folds each (I5: no resident Vec to push into —
            // the input `log` IS the durable source being replayed, not a resident copy).
            genesis: genesis_event.clone(),
            tip: genesis_event,
            spawned: Vec::new(),
            seeded_capabilities: false,
            close_outcome: None,
            kv: Kv::new(),
            next_effect_id: 0,
            settled: SettledSet::default(),
            open: BTreeMap::new(),
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
            // Reconstruct the open-obligation table + settled set + id counter from the log (§16c-S1/S5,
            // log-decouple I2). Rebuild the SAME OpenObligation the live `append` builds — recovery-
            // equivalent: replay(full) yields the identical table a live run + I6 checkpoint+tail produce.
            match &event.body {
                EventBody::Dispatched {
                    id,
                    schema_hash,
                    target,
                    token,
                    deadline_ms,
                    ..
                } => {
                    s.open.insert(
                        id.0,
                        OpenObligation {
                            target: target.clone(),
                            schema_hash: *schema_hash,
                            token: token.clone(),
                            // Same deadline-anchor carry as the append path (I5 step 2) — replay must
                            // rebuild the identical obligation, so the stall anchor survives recovery.
                            deadline_ms: *deadline_ms,
                            dispatch_hash: Some(event.hash()),
                            is_timer: false,
                        },
                    );
                    // Rebuild the resident seed flag (I3): same cause==genesis CAPABILITIES discriminant,
                    // now keyed on the CAPABILITIES family's schema-hash (schema-hash-only — the frame no
                    // longer carries the family string).
                    if *schema_hash == Some(capabilities_schema_hash())
                        && event.cause == Some(s.genesis.hash())
                    {
                        s.seeded_capabilities = true;
                    }
                    s.next_effect_id = s.next_effect_id.max(id.0 + 1);
                }
                EventBody::TimerArmed {
                    id,
                    deadline_ms,
                    token,
                    ..
                } => {
                    s.open.insert(
                        id.0,
                        OpenObligation {
                            target: std::sync::Arc::from(&b""[..]),
                            // Reserved TIMER schema-hash; never routed (is_timer is the discriminant).
                            schema_hash: Some(timer_schema_hash()),
                            token: token.clone(),
                            deadline_ms: Some(*deadline_ms),
                            dispatch_hash: None,
                            is_timer: true,
                        },
                    );
                    s.next_effect_id = s.next_effect_id.max(id.0 + 1);
                }
                EventBody::EffectResult { id, result, .. } => {
                    // Read the schema-hash BEFORE removing the obligation (dispatch_schema_hash_of reads the
                    // open table). Keyed on the NOW family's schema-hash (schema-hash-only).
                    let is_now = s.dispatch_schema_hash_of(*id) == Some(now_schema_hash());
                    s.open.remove(&id.0);
                    s.settled.insert(id.0);
                    s.next_effect_id = s.next_effect_id.max(id.0 + 1);
                    if is_now {
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
                    s.settled.insert(id.0);
                    s.next_effect_id = s.next_effect_id.max(id.0 + 1);
                }
                EventBody::AuthzDenied { id, .. } => {
                    s.next_effect_id = s.next_effect_id.max(id.0 + 1);
                }
                // Rebuild the resident lifecycle flags (I3) — same as live append.
                EventBody::Spawned { child_hash } => {
                    s.spawned.push(*child_hash);
                }
                EventBody::Closed { outcome } => {
                    s.close_outcome = Some(outcome.clone());
                }
                _ => {}
            }
            if observable(&event.body) {
                let _ = reducer.fold(&event, &mut s.kv).await;
            }
            // Advance the resident tip to the event just folded (log/state-decouple I1); after the loop,
            // tip is the last replayed event. No resident Vec to push into (I5) — the input `log` was the
            // durable source, already consumed by this loop.
            s.tip = event;
        }
        Ok(s)
    }

    /// The set of open (dispatched-but-unsettled) effect ids after recovery — what a driver must
    /// re-drive or time out (§16c-S1). Exposed for the recovery driver + tests.
    pub fn open_effect_ids(&self) -> Vec<EffectId> {
        self.open.keys().map(|n| EffectId(*n)).collect()
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
        reducer: &mut dyn Reducer,
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
        reducer: &mut dyn Reducer,
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
    /// The self-close [`CloseOutcome`] iff the session has closed (`state == Closed` via a self-close) —
    /// `Some` carries whether it ended in Success (with payload) or Failure (with reason), so an observer
    /// distinguishes the two closes that both collapse to `SessionState::Closed`. `None` for any non-closed
    /// state. Mirrors [`Session::close_outcome`].
    pub close_outcome: Option<CloseOutcome>,
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
    /// The in-flight effect's SCHEMA-HASH identity (schema-hash-only) — the durable-frame identity the open
    /// obligation carries. Observability only; a display layer resolves it to a name ([`schema_resolver`],
    /// display-only) or renders its base64url short form. Replaces the old `kind: &'static str` enum name.
    /// `None` for a register-by-string extension family with no wire hash yet (phase-3).
    pub schema_hash: Option<Hash>,
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
        EventBody::ChildCompleted { .. } => "ChildCompleted",
        EventBody::Checkpoint(_) => "Checkpoint",
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

fn observable(body: &EventBody) -> bool {
    match body {
        EventBody::Inbound { .. }
        | EventBody::EffectResult { .. }
        | EventBody::TimerFired { .. }
        | EventBody::AuthzDenied { .. }
        // ChildCompleted IS folded — the parent's SUPERVISOR reducer reacts to it per-child (restart /
        // count / route by child), unlike `Spawned` (a recorded edge, read from the log, not folded).
        | EventBody::ChildCompleted { .. }
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
        | EventBody::Spawned { .. }
        // A Checkpoint is a durable RESIDENT-STATE frame (GAP-4), appended by the prune path and consumed by
        // recover-from-checkpoint — never handed to the reducer's fold (like Genesis/Dispatched bookkeeping).
        | EventBody::Checkpoint(_) => false,
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
    buf.extend_from_slice(&req.target);
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
    use crate::test_log_source::*;

    // A reducer that, on an inbound message, publishes a semantic status to `public/` and arms a Timer
    // (an open obligation that stays unsettled — no executor call — so the session reads as Active).
    struct StatusReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for StatusReducer {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
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
        async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
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
        let captured = attach_recording_sink(&mut s);
        s.deliver(
            inbound(),
            None,
            &mut FailingReducer,
            &Authorizer::deny_all(),
            &mut exec,
        )
        .await
        .unwrap(); // deliver itself SUCCEEDS — the fold failure is data, not a kernel error.

        // A FoldFailed event is on the log, carrying the reason + BOTH cause linkages: the body's
        // `caused_event` field AND the envelope `Event.cause` edge (distinct — the body field is a payload,
        // `Event.cause` is the real causal-DAG parent edge replay/tamper-evidence/consumers walk; a regression
        // that filled one but not the other would break the DAG, so pin BOTH — liaison pr1963).
        let durable = replay_input(&captured);
        let (reason, body_caused, envelope_cause) = durable
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
        let inbound_hash = durable
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
        s.deliver(inbound(), None, &mut StatusReducer, &timer_cap(), &mut exec)
            .await
            .unwrap();
        assert!(
            s.kv().get(b"public/status").is_some(),
            "the session survives a failed fold and processes the next event"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn last_fault_reason_reads_the_tip_only_and_clears_when_the_session_progresses() {
        // The DERIVED accessor for the host status view's one steady-state fault read (log/state-decouple
        // I5: the host stops reaching into `log()` for it). Pins TIP-ONLY freshness: a session whose tip is
        // a FoldFailed reads Some(reason); once it progresses past the fault, it reads None again.
        let mut exec = RecordingExecutor::new();
        let mut s = Session::genesis(Hash::of(b"fail-v1"), Hash::of(b"test-spawn-nonce"));

        // Fresh (tip = Genesis): no fault.
        assert_eq!(
            s.last_fault_reason(),
            None,
            "a fresh session has no fault tip"
        );

        // Fold a failing inbound → tip becomes FoldFailed → the reason surfaces.
        s.deliver(
            inbound(),
            None,
            &mut FailingReducer,
            &Authorizer::deny_all(),
            &mut exec,
        )
        .await
        .unwrap();
        assert_eq!(
            s.last_fault_reason().as_deref(),
            Some("wasm reducer trapped: unreachable"),
            "with a FoldFailed tip the fault reason surfaces"
        );

        // A SUBSEQUENT successful fold moves the tip past the fault → the fault clears (freshest signal only).
        s.deliver(inbound(), None, &mut StatusReducer, &timer_cap(), &mut exec)
            .await
            .unwrap();
        assert_eq!(
            s.last_fault_reason(),
            None,
            "a FoldFailed the session progressed past is NOT reported — only the tip is the fresh signal"
        );
    }

    // A reducer that signals CLEAN SELF-COMPLETION on an inbound via FoldOutput::close(Success(payload)).
    struct ClosingReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for ClosingReducer {
        async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    FoldOutput::close(crate::event::CloseOutcome::Success(
                        crate::effect::Payload::Inline(b"done".to_vec().into()),
                    ))
                }
                _ => FoldOutput::none(),
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_reducer_close_signal_appends_closed_and_flips_is_closed() {
        // §6 supervision self-close TRIGGER: a reducer returning FoldOutput::close(outcome) makes the kernel
        // append a durable EventBody::Closed{outcome} — the ONLY production path to Closed (otherwise the
        // kernel recognizes it only on recovery) — and flips is_closed(), the hook a driver uses to route the
        // child-completed signal to a parent (§6). Terminal like FoldFailed: a closing fold routes no effects.
        let mut exec = RecordingExecutor::new();
        let mut s = Session::genesis(Hash::of(b"close-v1"), Hash::of(b"test-spawn-nonce"));
        let captured = attach_recording_sink(&mut s);

        assert!(!s.is_closed(), "a fresh session is not closed");
        s.deliver(
            inbound(),
            None,
            &mut ClosingReducer,
            &Authorizer::deny_all(),
            &mut exec,
        )
        .await
        .unwrap(); // deliver succeeds — a clean close is data, not a kernel error.

        assert!(s.is_closed(), "a close-signalling fold flips is_closed()");
        assert!(
            exec.seen.is_empty(),
            "a closing fold routes no effects (terminal)"
        );
        let durable = replay_input(&captured);
        let outcome = durable
            .iter()
            .rev()
            .find_map(|e| match &e.body {
                EventBody::Closed { outcome } => Some(outcome.clone()),
                _ => None,
            })
            .expect("a close-signalling fold appends a Closed event");
        assert_eq!(
            outcome,
            crate::event::CloseOutcome::Success(crate::effect::Payload::Inline(
                b"done".to_vec().into()
            )),
            "the Closed event carries the reducer's CloseOutcome verbatim"
        );
        // The outcome is also OBSERVABLE post-close off resident state (not just the durable log): the
        // close_outcome() accessor + the status_snapshot both carry it, so a driver sees HOW it closed.
        let expected = crate::event::CloseOutcome::Success(crate::effect::Payload::Inline(
            b"done".to_vec().into(),
        ));
        assert_eq!(
            s.close_outcome(),
            Some(&expected),
            "close_outcome() surfaces the reducer's chosen outcome post-close"
        );
        let snap = s.status_snapshot(None, 60_000);
        assert_eq!(snap.state, SessionState::Closed, "snapshot state is Closed");
        assert_eq!(
            snap.close_outcome,
            Some(expected),
            "the status snapshot carries the self-close outcome"
        );
    }

    // A reducer that self-closes with a FAILURE outcome — the discriminator that makes close_outcome()
    // load-bearing: a Success close and a Failure close BOTH collapse to SessionState::Closed, so only the
    // outcome tells them apart. Pins that a Failure self-close is observable AS a failure post-close
    // (unblocks the platform-conformance Success-vs-Failure close case).
    struct FailClosingReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for FailClosingReducer {
        async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => FoldOutput::close(
                    crate::event::CloseOutcome::Failure("goal unreachable".to_string()),
                ),
                _ => FoldOutput::none(),
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_failure_self_close_is_observable_as_failure_distinct_from_success() {
        let mut exec = RecordingExecutor::new();
        let mut s = Session::genesis(Hash::of(b"fail-close-v1"), Hash::of(b"test-spawn-nonce"));
        s.deliver(
            inbound(),
            None,
            &mut FailClosingReducer,
            &Authorizer::deny_all(),
            &mut exec,
        )
        .await
        .unwrap();

        assert!(
            s.is_closed(),
            "a Failure self-close still flips is_closed()"
        );
        let failure = crate::event::CloseOutcome::Failure("goal unreachable".to_string());
        assert_eq!(
            s.close_outcome(),
            Some(&failure),
            "close_outcome() carries the FAILURE outcome verbatim — distinct from a Success close"
        );
        let snap = s.status_snapshot(None, 60_000);
        assert_eq!(snap.state, SessionState::Closed);
        assert_eq!(
            snap.close_outcome,
            Some(failure),
            "the snapshot distinguishes a Failure close from a Success close"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_closed_session_refuses_further_delivery_keeping_the_terminal_tip_frozen() {
        // §6 terminal-tip invariant: once a reducer self-closes (FoldOutput::close → terminal Closed), a
        // further deliver() must be REFUSED (FoldRefused) — mirroring the is_terminated guard. Without it an
        // Inbound would fold + append PAST the Closed event, un-tailing it (a recovered tip would be that
        // Inbound, not Closed) and breaking replay-stability. (Reviewer finding on the supervision reap.)
        let mut exec = RecordingExecutor::new();
        let mut s = Session::genesis(Hash::of(b"close-guard-v1"), Hash::of(b"test-spawn-nonce"));
        s.deliver(
            inbound(),
            None,
            &mut ClosingReducer,
            &Authorizer::deny_all(),
            &mut exec,
        )
        .await
        .unwrap();
        assert!(s.is_closed(), "the fold self-closed the session");
        let count_at_close = s.event_count();

        // A second delivery to the CLOSED session is refused — no event appended, the Closed tip stays frozen.
        let refused = s
            .deliver(
                inbound(),
                None,
                &mut ClosingReducer,
                &Authorizer::deny_all(),
                &mut exec,
            )
            .await;
        assert!(
            matches!(refused, Err(KernelError::FoldRefused)),
            "deliver to a closed session is refused (FoldRefused), like a terminated one"
        );
        assert_eq!(
            s.event_count(),
            count_at_close,
            "no event was appended past the terminal Closed"
        );
        assert!(
            s.is_closed(),
            "still closed — the terminal tip is unchanged"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_closed_session_fires_no_pre_armed_timer_keeping_the_terminal_tip_frozen() {
        // §6: a timer armed by a PRIOR fold must not fire a TimerFired past the terminal Closed (same
        // un-tailing class as deliver). fire_due_timers refuses on a closed session, mirroring is_terminated.
        let mut exec = RecordingExecutor::new();
        let mut s = Session::genesis(Hash::of(b"close-timer-v1"), Hash::of(b"test-spawn-nonce"));
        // Arm a timer (StatusReducer arms one with absolute deadline 1000ms), then self-close.
        s.deliver(inbound(), None, &mut StatusReducer, &timer_cap(), &mut exec)
            .await
            .unwrap();
        assert!(
            s.status_snapshot(Some(2000), 60_000).armed_timers >= 1,
            "a timer is armed before the close"
        );
        s.deliver(
            inbound(),
            None,
            &mut ClosingReducer,
            &Authorizer::deny_all(),
            &mut exec,
        )
        .await
        .unwrap();
        assert!(
            s.is_closed(),
            "the session self-closed with a timer still armed"
        );
        let count_at_close = s.event_count();

        // The armed timer is now DUE (2000 >= 1000), but the closed session fires nothing.
        let fired = s
            .fire_due_timers(
                2000,
                &mut crate::reducer::InertReducer,
                &Authorizer::deny_all(),
                &mut exec,
            )
            .await;
        assert_eq!(fired, 0, "a closed session fires no timers");
        assert_eq!(
            s.event_count(),
            count_at_close,
            "no TimerFired was appended past the terminal Closed"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_closed_session_refuses_terminate_and_record_spawn_preserving_the_close_outcome() {
        // §6 terminal-tip completeness: a self-CLOSED session must also refuse terminate() and record_spawn()
        // (the two fold-free append seams) — appending Terminated or a Spawned edge past the terminal Closed
        // would un-tail it (double-terminal / clobbered CloseOutcome). This completes the is_closed guard
        // across ALL 6 is_terminated append sites.
        let mut exec = RecordingExecutor::new();
        let mut s = Session::genesis(Hash::of(b"close-term-v1"), Hash::of(b"test-spawn-nonce"));
        s.deliver(
            inbound(),
            None,
            &mut ClosingReducer,
            &Authorizer::deny_all(),
            &mut exec,
        )
        .await
        .unwrap();
        assert!(s.is_closed(), "the session self-closed");
        let count_at_close = s.event_count();
        let outcome_at_close = s.close_outcome().cloned();

        // terminate() on a closed session is refused — no Terminated marker past Closed.
        assert!(
            matches!(
                s.terminate(Hash::of(b"controller"), "cleanup".to_string())
                    .await,
                Err(KernelError::FoldRefused)
            ),
            "terminate() on a self-closed session is refused"
        );
        // record_spawn() likewise — no Spawned edge past Closed.
        assert!(
            matches!(
                s.record_spawn(Hash::of(b"a-child")).await,
                Err(KernelError::FoldRefused)
            ),
            "record_spawn() on a self-closed session is refused"
        );
        assert_eq!(
            s.event_count(),
            count_at_close,
            "no event was appended past the terminal Closed"
        );
        assert_eq!(
            s.close_outcome().cloned(),
            outcome_at_close,
            "the self-close CloseOutcome is preserved (terminate did not clobber it)"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_recovered_closed_session_stays_closed_with_its_outcome_and_refuses_appends() {
        // §6 replay-stability: a self-closed session must RECOVER from its durable log still closed, with its
        // CloseOutcome reconstructed (the Closed event's replay-apply re-captures close_outcome), so the
        // is_closed guards fire on the RECOVERED session too — a closed log is frozen across recovery, not
        // just live. (Pins the recovery side of the terminal-tip guard.)
        let mut exec = RecordingExecutor::new();
        let mut s = Session::genesis(
            Hash::of(b"recover-closed-v1"),
            Hash::of(b"test-spawn-nonce"),
        );
        let captured = attach_recording_sink(&mut s);
        s.deliver(
            inbound(),
            None,
            &mut ClosingReducer,
            &Authorizer::deny_all(),
            &mut exec,
        )
        .await
        .unwrap();
        let expected = s.close_outcome().cloned().expect("self-closed live");

        // Recover from the durable log (tail = Closed).
        let mut replayed = Session::replay(replay_input(&captured), &mut ClosingReducer)
            .await
            .expect("replay");
        assert!(
            replayed.is_closed(),
            "a recovered self-closed session is still closed"
        );
        assert_eq!(
            replayed.close_outcome(),
            Some(&expected),
            "the CloseOutcome is reconstructed on recovery, not just live"
        );
        let count_after_recover = replayed.event_count();

        // The is_closed guards fire on the RECOVERED session too — a further deliver is refused.
        let mut exec2 = RecordingExecutor::new();
        let refused = replayed
            .deliver(
                inbound(),
                None,
                &mut ClosingReducer,
                &Authorizer::deny_all(),
                &mut exec2,
            )
            .await;
        assert!(
            matches!(refused, Err(KernelError::FoldRefused)),
            "a recovered closed session refuses further delivery"
        );
        assert_eq!(
            replayed.event_count(),
            count_after_recover,
            "no event was appended past Closed on the recovered session"
        );
    }

    // A report-aware reducer (the fork-for-query summarize protocol, operator ruling (a)): on an ordinary
    // message it does work + publishes status; on the well-known `report` content-type it describes ITSELF
    // from LOCAL STATE (no model call — the operator's preferred path) by writing a summary to `public/`.
    // This is the generic-reducer shape a query fold uses: `if ct.is_report() { …summarize… }`.
    struct ReportingReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for ReportingReducer {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { content_type, .. } if content_type.is_report() => {
                    // Summarize from local KV alone — read the goal it recorded, describe progress.
                    let goal = kv
                        .get(b"private/goal")
                        .map(|v| String::from_utf8_lossy(&v).into_owned())
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
        async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
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
        let captured = attach_recording_sink(&mut s);
        s.deliver(
            inbound(),
            None,
            &mut PeerEmitterReducer { peer },
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
        assert_eq!(
            req.target_str().unwrap(),
            peer,
            "target is the peer session id"
        );
        assert_eq!(
            req.payload,
            Some(crate::effect::Payload::Inline(
                b"hello-peer".to_vec().into()
            )),
            "the message payload rides the emit"
        );

        // A durable Dispatched frame recorded the Emit schema-hash + the peer target (the crash-recovery-safe
        // record the host routes from — before the effect leaves the kernel). Schema-hash-only: the frame's
        // identity is the schema-hash (kind=Emit is gone), so match on the Emit built-in hash.
        let (schema_hash, target) = replay_input(&captured)
            .iter()
            .find_map(|e| match &e.body {
                EventBody::Dispatched {
                    schema_hash,
                    target,
                    ..
                } => Some((*schema_hash, target.clone())),
                _ => None,
            })
            .expect("a Dispatched frame was recorded for the emit");
        assert_eq!(
            schema_hash,
            Some(crate::ast_marshal::builtin_effect_schema_hash(
                &EffectKind::Emit
            )),
            "Dispatched records the Emit schema-hash"
        );
        assert_eq!(
            std::str::from_utf8(target.as_ref()).unwrap(),
            peer,
            "Dispatched records the peer target"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dispatched_frame_mirrors_the_effect_schema_hash_builtin_and_declared_family() {
        // schema-hash-only effect model: the durable Dispatched FRAME must MIRROR the effect's schema_hash
        // (populated at the kernel dispatch sites straight from `req.schema_hash`) — `Some(built-in hash)`
        // for a real EffectKind effect, and `Some(family hash)` for a well-known family with a DECLARED
        // schema. Pins that population so a refactor can't silently drop the schema identity off the durable
        // frame (the model keys the effect's identity on it). In-memory frame inspection = the fast
        // regression guard; v-compiler-ml's event_ast codec round-trip is the wire-level complement. Covers a
        // REAL routed dispatch (Emit) + a CONTROL inline dispatch (the capability seed) — both go through the
        // same `schema_hash: req.schema_hash` population arm. (control/capabilities gained a declared schema
        // in the 13-family target-OUT slice, so its frame now carries Some, not None — a family with NO
        // declared schema yet, e.g. store/*, would still be None.)

        // (1) REAL effect (routed dispatch): a peer-directed Emit → frame schema_hash = Some(built-in Emit).
        let peer = "session-B";
        let mut exec = RecordingExecutor::new();
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Emit,
            predicate: ResourcePredicate::Exact(peer.into()),
        }]);
        let mut s = Session::genesis(Hash::of(b"emitter-v1"), Hash::of(b"test-spawn-nonce"));
        let captured = attach_recording_sink(&mut s);
        s.deliver(
            inbound(),
            None,
            &mut PeerEmitterReducer { peer },
            &authz,
            &mut exec,
        )
        .await
        .expect("deliver");
        let emit_schema = replay_input(&captured)
            .iter()
            .find_map(|e| match &e.body {
                EventBody::Dispatched { schema_hash, .. }
                    if *schema_hash
                        == Some(crate::ast_marshal::builtin_effect_schema_hash(
                            &EffectKind::Emit,
                        )) =>
                {
                    Some(*schema_hash)
                }
                _ => None,
            })
            .expect("an Emit Dispatched frame was recorded");
        assert_eq!(
            emit_schema,
            Some(crate::ast_marshal::builtin_effect_schema_hash(
                &EffectKind::Emit
            )),
            "a real effect's Dispatched frame mirrors its built-in schema_hash"
        );

        // (2) CONTROL family with a DECLARED schema (inline dispatch): the capability seed's
        // control/capabilities frame → schema_hash Some(family hash). control/capabilities gained a schema in
        // the 13-family target-OUT slice, so the frame now mirrors it (computed from the FAMILY, not the
        // Emit-placeholder kind). Populated straight from req.schema_hash, same as the real-effect arm.
        let mut exec2 = RecordingExecutor::new();
        let mut s2 = Session::genesis(Hash::of(b"seeded-v1"), Hash::of(b"test-spawn-nonce"));
        let captured2 = attach_recording_sink(&mut s2);
        s2.seed_capabilities(
            &mut crate::reducer::InertReducer,
            &Authorizer::deny_all(),
            &mut exec2,
        )
        .await;
        // The CAPABILITIES seed frame's identity is the CAPABILITIES family schema-hash (schema-hash-only —
        // the frame no longer carries the family string; identity IS the hash). Find the seed's Dispatched
        // frame by that hash and confirm it was recorded.
        let expected =
            crate::ast_marshal::family_effect_schema_hash(crate::effect::effect_ct::CAPABILITIES)
                .expect("the well-known CAPABILITIES family has a declared schema");
        let caps_schema = replay_input(&captured2)
            .iter()
            .find_map(|e| match &e.body {
                EventBody::Dispatched { schema_hash, .. } if *schema_hash == Some(expected) => {
                    Some(*schema_hash)
                }
                _ => None,
            })
            .expect("a control/capabilities Dispatched frame (keyed by its schema-hash) was recorded by the seed");
        // Identity check: the recorded frame's schema-hash IS the CAPABILITIES family hash (the seq-39
        // family-string identity is gone — the durable frame carries only this hash).
        assert_eq!(
            caps_schema,
            Some(expected),
            "the seed's control/capabilities frame carries the CAPABILITIES family schema-hash"
        );
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
        let captured = attach_recording_sink(&mut s);
        s.deliver(
            inbound(),
            None,
            &mut PeerEmitterReducer { peer: "session-C" }, // NOT the granted target
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
            !replay_input(&captured)
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
        s.deliver(inbound(), None, &mut StatusReducer, &timer_cap(), &mut exec)
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
        s.deliver(inbound(), None, &mut StatusReducer, &timer_cap(), &mut exec)
            .await
            .unwrap();

        // The parent is Active with one armed timer and its published status set.
        let parent_events_before = s.event_count();
        let parent_snap_before = s.status_snapshot(Some(500), 300_000);
        assert_eq!(parent_snap_before.state, SessionState::Active);
        assert_eq!(parent_snap_before.armed_timers, 1);

        // Fork it: the fork inherits the materialized KV (incl. the `public/` status) but starts as a
        // clean reactive session — its own genesis, NO inherited in-flight obligations or armed timers.
        let fork = s.fork_for_query();
        assert_eq!(fork.event_count(), 1); // just the fork's own genesis
        assert_eq!(fork.open_effects(), 0); // did NOT inherit the parent's open timer obligation
        assert_eq!(fork.next_timer_deadline(), None);
        // Same reducer-hash (folds identically) — the snapshot descriptor proves it.
        assert_eq!(fork.snapshot().reducer, s.snapshot().reducer);
        // The KV came across: the fork can read what the parent published.
        assert_eq!(
            fork.kv().get(b"public/status").as_deref(),
            Some(&b"investigating auth"[..])
        );
        // And the private key too (the fork is a full-privilege clone of the materialized state; scoping
        // is the caller's capability concern, not the KV's — the fork just has the same KV).
        assert_eq!(
            fork.kv().get(b"private/secret").as_deref(),
            Some(&b"nope"[..])
        );

        // NON-INTERFERENCE: forking read the parent immutably — its log, timers, and state are unchanged.
        assert_eq!(s.event_count(), parent_events_before);
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
                &mut StatusReducer,
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
                &mut StatusReducer,
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
        assert_eq!(async_s.event_count(), sync_s.event_count());
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
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
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
        let mut reducer = TimerThenPublishReducer;
        let mut exec = RecordingExecutor::new();
        let mut s = Session::genesis(Hash::of(b"timer-v1"), Hash::of(b"test-spawn-nonce"));
        s.deliver(inbound(), None, &mut reducer, &timer_cap(), &mut exec)
            .await
            .unwrap();
        // Armed, not yet fired.
        assert_eq!(s.next_timer_deadline(), Some(1000));
        assert!(s.kv().get(b"public/woke").is_none());

        // Fire everything due at now=1500 (past the 1000ms deadline).
        let fired = s
            .fire_due_timers(1500, &mut reducer, &timer_cap(), &mut exec)
            .await;
        assert_eq!(fired, 1, "exactly one timer was due and fired");
        // The reducer woke on the TimerFired and published its marker.
        assert_eq!(
            s.kv().get(b"public/woke").as_deref(),
            Some(&b"timer-fired"[..])
        );
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
        s.deliver(inbound(), None, &mut StatusReducer, &timer_cap(), &mut exec)
            .await
            .unwrap();
        let parent_events = s.event_count();

        let mut fork = s.fork_for_query();
        let mut fork_exec = RecordingExecutor::new();
        fork.deliver(
            inbound(),
            None,
            &mut StatusReducer,
            &timer_cap(),
            &mut fork_exec,
        )
        .await
        .unwrap();
        // The fork folded the query and did work in its OWN log.
        assert!(fork.event_count() > 1);
        assert_eq!(fork.status_snapshot(Some(0), 300_000).armed_timers, 1);

        // The parent's log length is untouched by anything the fork did.
        assert_eq!(s.event_count(), parent_events);
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
        live.deliver(
            inbound(),
            None,
            &mut ReportingReducer,
            &timer_cap(),
            &mut exec,
        )
        .await
        .unwrap();
        let live_events_before = live.event_count();
        assert_eq!(
            live.kv().get(b"private/goal").as_deref(),
            Some(&b"the auth module"[..])
        );

        // Operator asks "what is this session doing?" → fork it and deliver a report query.
        let mut fork = live.fork_for_query();
        let mut fork_exec = RecordingExecutor::new();
        fork.deliver(
            report_inbound(),
            None,
            &mut ReportingReducer,
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
        assert_eq!(live.event_count(), live_events_before);
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
    use crate::test_log_source::*;

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
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
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
        async fn perform(
            &mut self,
            _id: EffectId,
            req: &EffectRequest,
            _key: Hash,
        ) -> EffectOutcome {
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
            .map(|(_, v)| u64::from_le_bytes(<[u8; 8]>::try_from(v.as_ref()).unwrap()))
            .collect()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn now_sequence_is_strictly_increasing_even_from_a_stuck_clock() {
        let mut exec = StuckClock(1000); // same raw reading every time
        let mut s = Session::genesis(Hash::of(b"now-v1"), Hash::of(b"test-spawn-nonce"));
        s.deliver(inbound(), None, &mut NowReducer, &now_cap(), &mut exec)
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
        let captured = attach_recording_sink(&mut s);
        s.deliver(inbound(), None, &mut NowReducer, &now_cap(), &mut exec)
            .await
            .unwrap();
        let live_seq = recorded_now_sequence(&s);
        let live_last_now = s.last_now;

        // Replay the log READ BACK FROM THE DURABLE SOURCE (the sink), not the resident Vec — recovery
        // reconstructs from what was persisted. The recorded (already-clamped) Now results must rebuild
        // the SAME last_now + the SAME sequence — replay never re-clamps, it re-derives (determinism).
        let log = replay_input(&captured);
        let replayed = Session::replay(log, &mut NowReducer).await.expect("replay");
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
        let captured = attach_recording_sink(&mut s);
        s.deliver(inbound(), None, &mut NowReducer, &now_cap(), &mut exec)
            .await
            .unwrap();
        let log = replay_input(&captured);

        let replayed = Session::replay(log.clone(), &mut NowReducer)
            .await
            .expect("replay");
        // Reconstructs the live session's derived state exactly.
        assert_eq!(replayed.snapshot().kv_root, s.snapshot().kv_root);
        assert_eq!(replayed.last_now, s.last_now);
        assert_eq!(replayed.last_now, 1002);
        assert_eq!(replayed.event_count(), s.event_count());
        assert_eq!(recorded_now_sequence(&replayed), recorded_now_sequence(&s));

        // Two replays of the same log agree — the re-fold has no hidden nondeterminism.
        let replayed2 = Session::replay(log, &mut NowReducer)
            .await
            .expect("replay 2");
        assert_eq!(replayed2.snapshot().kv_root, replayed.snapshot().kv_root);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn open_obligation_table_serves_accessors_and_replay_rebuilds_it_identically() {
        // log-decouple I2: the resident open-obligation table serves dispatch_hash_of / dispatch_token_of /
        // dispatch_family_of with ZERO log access, folds armed timers in (is_timer + deadline), and replay
        // rebuilds the IDENTICAL table (recovery-equivalence for the open set). Drive a reducer that emits a
        // tokened Http effect on inbound; the effect is OPEN until its result folds.
        struct EmitTokenedHttp;
        #[async_trait::async_trait(?Send)]
        impl Reducer for EmitTokenedHttp {
            async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
                match &event.body {
                    EventBody::Inbound { .. } => FoldOutput::with_effects(vec![Effect {
                        request: EffectRequest::new(
                            EffectKind::Http,
                            "https://ok.host/x",
                            None,
                            Timeliness::Interactive,
                        ),
                        token: Some(b"cont-1".to_vec()),
                    }]),
                    _ => FoldOutput::none(),
                }
            }
        }
        // A never-answering executor so the Http effect stays OPEN after drive (obligation retained).
        struct NeverExec;
        #[async_trait::async_trait(?Send)]
        impl Executor for NeverExec {
            async fn perform(
                &mut self,
                _id: EffectId,
                _req: &EffectRequest,
                _key: Hash,
            ) -> EffectOutcome {
                EffectOutcome::Deferred
            }
        }
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Http,
            predicate: crate::effect::ResourcePredicate::HostIn(vec!["ok.host".into()]),
        }]);
        let mut s = Session::genesis(Hash::of(b"i2-obl"), Hash::of(b"test-spawn-nonce"));
        let captured = attach_recording_sink(&mut s);
        s.deliver(
            inbound(),
            None,
            &mut EmitTokenedHttp,
            &authz,
            &mut NeverExec,
        )
        .await
        .unwrap();

        // Exactly one open obligation (the deferred Http), id 0.
        let ids = s.open_effect_ids();
        assert_eq!(ids, vec![EffectId(0)]);
        // The table serves all three accessors with no log scan.
        assert_eq!(
            s.dispatch_schema_hash_of(EffectId(0)),
            Some(crate::ast_marshal::builtin_effect_schema_hash(
                &EffectKind::Http
            ))
        );
        assert_eq!(
            s.dispatch_token_of(EffectId(0)),
            Some(Some(b"cont-1".to_vec()))
        );
        assert!(s.dispatch_hash_of(EffectId(0)).is_some());

        // Replay rebuilds the IDENTICAL open table (recovery-equivalence): same open ids + same
        // per-obligation schema-hash/token/hash. Read the log from the durable source (sink), not the Vec.
        let log = replay_input(&captured);
        let replayed = Session::replay(log, &mut EmitTokenedHttp)
            .await
            .expect("replay");
        assert_eq!(replayed.open_effect_ids(), s.open_effect_ids());
        assert_eq!(
            replayed.dispatch_schema_hash_of(EffectId(0)),
            s.dispatch_schema_hash_of(EffectId(0))
        );
        assert_eq!(
            replayed.dispatch_token_of(EffectId(0)),
            s.dispatch_token_of(EffectId(0))
        );
        assert_eq!(
            replayed.dispatch_hash_of(EffectId(0)),
            s.dispatch_hash_of(EffectId(0))
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn build_checkpoint_descriptor_captures_live_derived_state() {
        // GAP-4 increment #2: `build_checkpoint_descriptor` captures ALL log-derived resident state so a
        // checkpoint@N lets recovery resume from `[Genesis, Checkpoint, tail]` WITHOUT the pruned prefix.
        // Drive ONE tokened, deferred (still-open) Http effect, then assert the descriptor mirrors the live
        // session's derived state: kv_root, the id counter, the clock, the settled watermark/exceptions, the
        // open obligation (WITH its map-key id + schema-hash/token/is_timer/dispatch_hash), spawned, close.
        struct EmitTokenedHttp;
        #[async_trait::async_trait(?Send)]
        impl Reducer for EmitTokenedHttp {
            async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
                match &event.body {
                    EventBody::Inbound { .. } => FoldOutput::with_effects(vec![Effect {
                        request: EffectRequest::new(
                            EffectKind::Http,
                            "https://ok.host/x",
                            None,
                            Timeliness::Interactive,
                        ),
                        token: Some(b"cont-1".to_vec()),
                    }]),
                    _ => FoldOutput::none(),
                }
            }
        }
        // A never-answering executor so the Http effect stays OPEN (its obligation is retained for the descriptor).
        struct NeverExec;
        #[async_trait::async_trait(?Send)]
        impl Executor for NeverExec {
            async fn perform(
                &mut self,
                _id: EffectId,
                _req: &EffectRequest,
                _key: Hash,
            ) -> EffectOutcome {
                EffectOutcome::Deferred
            }
        }
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Http,
            predicate: crate::effect::ResourcePredicate::HostIn(vec!["ok.host".into()]),
        }]);
        let mut s = Session::genesis(Hash::of(b"gap4-ckpt"), Hash::of(b"test-spawn-nonce"));
        s.deliver(
            inbound(),
            None,
            &mut EmitTokenedHttp,
            &authz,
            &mut NeverExec,
        )
        .await
        .unwrap();

        let d = s.build_checkpoint_descriptor();

        // Derived scalar state mirrors the live session.
        assert_eq!(
            d.kv_root,
            s.snapshot().kv_root,
            "kv_root == live snapshot root"
        );
        assert_eq!(d.next_effect_id, 1, "one effect dispatched → next id is 1");
        assert_eq!(d.last_now, s.last_now);
        // Nothing settled yet (the Http is deferred/open): watermark 0, no exceptions.
        assert_eq!(d.settled_watermark, 0);
        assert!(d.settled_exceptions.is_empty());

        // The one open obligation, captured WITH its map-key id and frame fields.
        assert_eq!(d.open.len(), 1, "one open (deferred Http) obligation");
        let ob = &d.open[0];
        assert_eq!(ob.id, 0);
        assert_eq!(ob.token.as_deref(), Some(&b"cont-1"[..]));
        assert!(!ob.is_timer, "an Http effect obligation is not a timer");
        assert!(
            ob.dispatch_hash.is_some(),
            "a dispatched effect carries its Dispatched-frame hash"
        );
        assert_eq!(
            ob.schema_hash,
            Some(crate::ast_marshal::builtin_effect_schema_hash(
                &EffectKind::Http
            )),
            "the obligation's schema-hash identity survives into the descriptor"
        );

        // No lifecycle state yet.
        assert!(d.spawned.is_empty());
        assert!(d.close_outcome.is_none());

        // The descriptor's open set mirrors the resident open table 1:1 (same ids) — the recovery-equivalence
        // property the later log-prune / recover-from-checkpoint increments rely on.
        assert_eq!(
            d.open.iter().map(|o| o.id).collect::<Vec<_>>(),
            s.open_effect_ids()
                .into_iter()
                .map(|EffectId(i)| i)
                .collect::<Vec<_>>(),
        );
    }

    // A reducer that, on an inbound message, emits a `control/summary` effect carrying summary bytes in
    // its payload (the fork-for-query control-plane pattern). It's a control/* family → authz-exempt +
    // host-surfaced (register-by-string beat 3).
    struct SummaryEmitReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for SummaryEmitReducer {
        async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
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
        let captured = attach_recording_sink(&mut session);
        let control = session
            .deliver_control(
                inbound(),
                None,
                &mut SummaryEmitReducer,
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
            !replay_input(&captured)
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
            &mut SummaryEmitReducer,
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
        async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
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
        let captured = attach_recording_sink(&mut session);
        // Grant the emit family (any resource) so the REGULAR effect authorizes; control is exempt anyway.
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Emit,
            predicate: crate::effect::ResourcePredicate::Any,
        }]);
        let control = session
            .deliver_control(inbound(), None, &mut MixedEmitReducer, &authz, &mut exec)
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
        assert_eq!(exec.seen[0].0.target_str().unwrap(), "world");
        assert_eq!(
            exec.seen[0].0.content_type.family.as_ref(),
            crate::effect::effect_ct::EMIT
        );

        // The regular effect was authorized (granted) → no AuthzDenied event for it, and it produced a
        // dispatch. (Control being exempt also produces no AuthzDenied — so zero denials total here.)
        assert!(
            !replay_input(&captured)
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
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    // Build via new_with_family so the family AND its schema-hash are CONSISTENT (the
                    // schema-hash-only identity): hand-patching content_type.family after new(Emit) would
                    // leave schema_hash as Emit's, which the durable frame now records as the identity.
                    let request = EffectRequest::new_with_family(
                        crate::effect::effect_ct::SIGNATURE,
                        "component-hash-abc",
                        None,
                        Timeliness::Interactive,
                    );
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
    async fn control_signature_is_surfaced_dispatched_and_settle_effect_result_resumes_the_guest() {
        // The fold-back control pattern (control/signature = the THIRD control disposition): unlike
        // capabilities (kernel-answered inline) or summary (fire-and-forget fork-scrape, no Dispatched), a
        // signature query is SURFACED to the driver AND given a Dispatched frame, so it is OPEN and awaiting a
        // HOST answer that must resume the emitting reducer. Prove the whole loop: surface → open+dispatched →
        // settle_effect_result folds the descriptor back → the guest's continuation resumes (writes KV).
        let mut exec = RecordingExecutor::new();
        let mut session = Session::genesis(Hash::of(b"sig-v1"), Hash::of(b"nonce"));
        let captured = attach_recording_sink(&mut session);
        let control = session
            .deliver_control(
                inbound(),
                None,
                &mut SignatureQueryReducer,
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
        let durable = replay_input(&captured);
        let dispatched_schema = durable
            .iter()
            .find_map(|e| match &e.body {
                EventBody::Dispatched { schema_hash, .. } => Some(*schema_hash),
                _ => None,
            })
            .expect("a Dispatched frame was recorded for the fold-back control");
        assert_eq!(
            dispatched_schema,
            Some(
                crate::ast_marshal::family_effect_schema_hash(crate::effect::effect_ct::SIGNATURE)
                    .expect("control/signature has a declared schema-hash")
            ),
            "the dispatch records the control/signature schema-hash (so recovery classifies it by identity)"
        );
        // No AuthzDenied — control is exempt even under deny_all (a routed effect would be denied).
        assert!(!durable
            .iter()
            .any(|e| matches!(e.body, EventBody::AuthzDenied { .. })));

        // The HOST reflects the target + settles the query with the descriptor bytes → the guest resumes.
        let descriptor = b"(component-signature (export (name run)))".to_vec();
        let settled = session
            .settle_effect_result(
                sig_id,
                EffectOutcome::Ok(Some(crate::effect::Payload::Inline(
                    descriptor.clone().into(),
                ))),
                &mut SignatureQueryReducer,
                &Authorizer::deny_all(),
                &mut exec,
            )
            .await;
        assert!(settled, "settling an open fold-back control returns true");
        // The continuation resumed: the reducer folded the EffectResult + wrote the descriptor to KV.
        assert_eq!(
            session.kv().get(b"sig/descriptor").map(|b| b.to_vec()),
            Some(descriptor),
            "settle_effect_result folds the descriptor back → the emitting reducer resumes with it"
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
            .settle_effect_result(
                sig_id,
                EffectOutcome::Ok(Some(crate::effect::Payload::Inline(
                    b"other".to_vec().into(),
                ))),
                &mut SignatureQueryReducer,
                &Authorizer::deny_all(),
                &mut exec,
            )
            .await;
        assert!(
            !dup,
            "a duplicate settle of an already-settled id is a no-op"
        );
        // Re-read the durable log (the sink kept capturing through the settle) — exactly one EffectResult.
        let result_count = replay_input(&captured)
            .iter()
            .filter(|e| matches!(e.body, EventBody::EffectResult { .. }))
            .count();
        assert_eq!(
            result_count, 1,
            "exactly one EffectResult — the dup settle appended nothing"
        );

        // Settling an id that was never dispatched is likewise a no-op (nothing open to resume).
        let never = session
            .settle_effect_result(
                EffectId(9999),
                EffectOutcome::Ok(None),
                &mut SignatureQueryReducer,
                &Authorizer::deny_all(),
                &mut exec,
            )
            .await;
        assert!(!never, "settling an unknown/never-dispatched id is a no-op");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn settle_effect_result_err_path_resumes_the_guest_with_a_failure() {
        // The host couldn't reflect the target (bad bytes, missing blob) → it settles with an Err. The
        // reducer's continuation resumes on the err arm (writes sig/error), same as a failed routed effect —
        // never stuck. Proves the fold-back seam carries a failure as cleanly as a success.
        let mut exec = RecordingExecutor::new();
        let mut session = Session::genesis(Hash::of(b"sig-err-v1"), Hash::of(b"nonce"));
        let control = session
            .deliver_control(
                inbound(),
                None,
                &mut SignatureQueryReducer,
                &Authorizer::deny_all(),
                &mut exec,
            )
            .await
            .expect("deliver");
        let sig_id = control[0].id;
        let settled = session
            .settle_effect_result(
                sig_id,
                EffectOutcome::err("not a valid component".to_string()),
                &mut SignatureQueryReducer,
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
    async fn common_deliver_fail_safes_a_dropped_fold_back_control_instead_of_orphaning_it() {
        // FAIL-SAFE (reviewer LOW, latent): a fold-back control (control/signature) gets a Dispatched frame
        // so it's OPEN awaiting a host settle — but the common `deliver` (the default live path) DROPS the
        // ControlEffect, so a signature-querier run on `deliver` (instead of `deliver_control`) would ORPHAN:
        // open forever, continuation never resumes. Fix: `deliver` settles each dropped fold-back control with
        // an Err so the reducer resumes on its err arm rather than orphaning. Prove it: emit control/signature
        // through the COMMON deliver → the effect is NOT left open + the reducer resumed (wrote sig/error).
        let mut exec = RecordingExecutor::new();
        let mut session =
            Session::genesis(Hash::of(b"sig-deliver-failsafe-v1"), Hash::of(b"nonce"));
        let captured = attach_recording_sink(&mut session);
        session
            .deliver(
                inbound(),
                None,
                &mut SignatureQueryReducer,
                &Authorizer::deny_all(),
                &mut exec,
            )
            .await
            .expect("deliver");
        // NOT orphaned: the dropped fold-back control was settled, so nothing stays open.
        assert_eq!(
            session.open_effects(),
            0,
            "a fold-back control dropped by the common deliver must be settled, not left open forever"
        );
        // The reducer RESUMED on the err arm (the fail-safe settles Err) — never stuck.
        assert_eq!(
            session.kv().get(b"sig/error").map(|b| b.to_vec()),
            Some(b"query-failed".to_vec()),
            "the fail-safe Err settle resumes the guest's continuation on the drop-control path"
        );
        // Exactly one EffectResult (the fail-safe settle) — the effect is durably settled, not phantom-open.
        assert_eq!(
            replay_input(&captured)
                .iter()
                .filter(|e| matches!(e.body, EventBody::EffectResult { .. }))
                .count(),
            1,
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn common_deliver_does_not_disturb_a_fire_and_forget_summary() {
        // The fail-safe is SCOPED to fold-back controls — control/summary (fire-and-forget, no Dispatched
        // frame, never open) is simply dropped by the common deliver with nothing to settle. Guard that the
        // fail-safe loop doesn't append a spurious EffectResult for a summary (which has no open effect).
        let mut exec = RecordingExecutor::new();
        let mut session = Session::genesis(Hash::of(b"sum-deliver-v1"), Hash::of(b"nonce"));
        let captured = attach_recording_sink(&mut session);
        session
            .deliver(
                inbound(),
                None,
                &mut SummaryEmitReducer,
                &Authorizer::deny_all(),
                &mut exec,
            )
            .await
            .expect("deliver");
        assert_eq!(
            session.open_effects(),
            0,
            "a fire-and-forget summary never opens an effect"
        );
        assert!(
            !replay_input(&captured)
                .iter()
                .any(|e| matches!(e.body, EventBody::EffectResult { .. })),
            "no EffectResult for a summary — the fail-safe is scoped to fold-back controls only"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn control_summary_still_has_no_dispatched_frame_and_is_not_settleable() {
        // Guard the SELECTIVE dispatch: only a fold-back control (signature) gets a Dispatched frame; summary
        // stays fire-and-forget (NO frame, never OPEN). A regression that dispatched summary too would leave
        // it hanging as a never-settled open effect. So after surfacing a summary, open_effect_count is 0 and
        // there is no Dispatched frame — and settling its (non-open) id is a no-op.
        let mut exec = RecordingExecutor::new();
        let mut session = Session::genesis(Hash::of(b"sum-nodispatch-v1"), Hash::of(b"nonce"));
        let captured = attach_recording_sink(&mut session);
        let control = session
            .deliver_control(
                inbound(),
                None,
                &mut SummaryEmitReducer,
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
            !replay_input(&captured)
                .iter()
                .any(|e| matches!(e.body, EventBody::Dispatched { .. })),
            "no Dispatched frame for a fire-and-forget summary (only fold-back controls dispatch)"
        );
        // Settling the summary's id is a no-op (it was never opened) — no phantom EffectResult.
        let settled = session
            .settle_effect_result(
                control[0].id,
                EffectOutcome::Ok(None),
                &mut SummaryEmitReducer,
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
        async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    // Build via new_with_family so family + schema-hash are CONSISTENT (schema-hash-only
                    // identity — hand-patching family after new(Emit) leaves schema_hash as Emit's).
                    let request = EffectRequest::new_with_family(
                        crate::effect::effect_ct::CAPABILITIES,
                        "self",
                        None,
                        Timeliness::Interactive,
                    );
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
        let captured = attach_recording_sink(&mut session);
        let control = session
            .deliver_control(
                inbound(),
                None,
                &mut CapabilitiesQueryReducer,
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
        let durable = replay_input(&captured);
        let dispatched_schema = durable
            .iter()
            .find_map(|e| match &e.body {
                EventBody::Dispatched { schema_hash, .. } => Some(*schema_hash),
                _ => None,
            })
            .expect("a Dispatched frame was recorded");
        assert_eq!(
            dispatched_schema,
            Some(
                crate::ast_marshal::family_effect_schema_hash(crate::effect::effect_ct::CAPABILITIES)
                    .unwrap()
            ),
            "the inline-capabilities dispatch records the control/capabilities schema-hash (its identity)"
        );

        // The kernel folded an EffectResult carrying the manifest bytes. Find it + decode.
        let payload = durable
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
        async fn fold(&mut self, _event: &Event, _kv: &mut Kv) -> FoldOutput {
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
        let captured = attach_recording_sink(&mut session);
        // Precondition: a bare genesis log has exactly the Genesis event, no manifest yet.
        assert_eq!(session.event_count(), 1);

        let surfaced = session
            .seed_capabilities(&mut InertReducer, &authz, &mut exec)
            .await;
        // The seed answers inline — nothing surfaces to the driver, nothing routed to the executor.
        assert!(
            surfaced.is_empty(),
            "the seed is answered inline, not surfaced"
        );

        // The seed's durable Dispatched records the control family (recovery-classifiable), cause-linked
        // to genesis.
        let durable = replay_input(&captured);
        let dispatched_schema = durable
            .iter()
            .find_map(|e| match &e.body {
                EventBody::Dispatched { schema_hash, .. } => Some(*schema_hash),
                _ => None,
            })
            .expect("the seed recorded a Dispatched frame");
        assert_eq!(
            dispatched_schema,
            Some(
                crate::ast_marshal::family_effect_schema_hash(
                    crate::effect::effect_ct::CAPABILITIES
                )
                .unwrap()
            ),
            "the seed dispatch is classifiable by the control/capabilities schema-hash on recovery"
        );

        // Born knowing: a capabilities-manifest EffectResult is in the log after the seed, decodable, with
        // the served+granted emit family reading granted.
        let payload = durable
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
        let captured = attach_recording_sink(&mut session);

        // Seed the baseline manifest with an emit-only executor (http is Absent — no executor serves it).
        let mut narrow = CompositeExecutor::new().with_effect(
            crate::effect::effect_ct::EMIT,
            Box::new(RecordingExecutor::new()),
        );
        session
            .seed_capabilities(&mut InertReducer, &authz, &mut narrow)
            .await;
        // Count the manifest EffectResults on the DURABLE log (read from the sink each call — it keeps
        // capturing as the seed + push fold more results through append).
        let cap_results = || {
            replay_input(&captured)
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
        assert_eq!(cap_results(), 1, "seed folded one manifest");

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
            .push_capabilities_changed(&mut InertReducer, &authz, &mut wide)
            .await;
        assert!(
            surfaced.is_empty(),
            "the push is answered inline, not surfaced"
        );
        assert_eq!(
            cap_results(),
            2,
            "a real surface change folds a second (capabilities-changed) manifest"
        );
        // The pushed manifest reads http as granted now (served + permitted).
        let latest_payload = replay_input(&captured)
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
        let log_len = session.event_count();
        let noop = session
            .push_capabilities_changed(&mut InertReducer, &authz, &mut wide)
            .await;
        assert!(noop.is_empty());
        assert_eq!(
            session.event_count(),
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
        let captured = attach_recording_sink(&mut session);

        let first = session
            .seed_capabilities(&mut InertReducer, &authz, &mut exec)
            .await;
        assert!(first.is_empty(), "seed answered inline");
        let after_first = session.event_count();
        // Exactly one control/capabilities dispatch after the first seed — counted on the DURABLE log
        // (read from the sink each call).
        let cap_dispatches = || {
            replay_input(&captured)
                .iter()
                .filter(|e| {
                    matches!(&e.body, EventBody::Dispatched { schema_hash, .. }
                        if *schema_hash == Some(crate::ast_marshal::family_effect_schema_hash(crate::effect::effect_ct::CAPABILITIES).unwrap()))
                })
                .count()
        };
        assert_eq!(cap_dispatches(), 1, "one seed dispatch after first call");

        // Second call: no-op — empty return, log UNCHANGED, still exactly one seed dispatch.
        let second = session
            .seed_capabilities(&mut InertReducer, &authz, &mut exec)
            .await;
        assert!(second.is_empty(), "a repeat seed is a no-op");
        assert_eq!(
            session.event_count(),
            after_first,
            "a repeat seed appends nothing to the log"
        );
        assert_eq!(
            cap_dispatches(),
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
        let captured = attach_recording_sink(&mut session);

        // Guest issues a control/capabilities query (via an inbound-triggered fold) BEFORE any seed.
        session
            .deliver(
                inbound(),
                None,
                &mut CapabilitiesQueryReducer,
                &authz,
                &mut exec,
            )
            .await
            .expect("guest query");
        // Count capabilities dispatches on the DURABLE log (read from the sink each call).
        let cap_dispatches = || {
            replay_input(&captured)
                .iter()
                .filter(|e| {
                    matches!(&e.body, EventBody::Dispatched { schema_hash, .. }
                        if *schema_hash == Some(crate::ast_marshal::family_effect_schema_hash(crate::effect::effect_ct::CAPABILITIES).unwrap()))
                })
                .count()
        };
        assert_eq!(
            cap_dispatches(),
            1,
            "the guest query dispatched one capabilities frame"
        );

        // Now seed — the guard must NOT be fooled by the guest's frame; the seed must still fire.
        session
            .seed_capabilities(&mut InertReducer, &authz, &mut exec)
            .await;
        assert_eq!(
            cap_dispatches(),
            2,
            "the seed fires despite a prior guest capabilities query (guard keys on cause==genesis)"
        );
        // And the genesis-caused (seed) frame is present exactly once.
        let genesis_hash = session.genesis_ref().hash();
        let seed_frames = replay_input(&captured)
            .iter()
            .filter(|e| {
                matches!(&e.body, EventBody::Dispatched { schema_hash, .. }
                    if *schema_hash == Some(crate::ast_marshal::family_effect_schema_hash(crate::effect::effect_ct::CAPABILITIES).unwrap()))
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
        let captured = attach_recording_sink(&mut session);
        session
            .seed_capabilities(&mut InertReducer, &authz, &mut exec)
            .await;

        // Precondition: after seeding, the seed dispatch is already settled (result folded), nothing open.
        assert_eq!(
            session.open_effects(),
            0,
            "the seed's dispatch is settled by its answer — no open in-flight obligation"
        );
        let live_root = session.snapshot().kv_root;
        let live_len = session.event_count();

        // Replay the durable log READ BACK FROM THE SOURCE (sink) — recovery reconstructs the same session.
        let log = replay_input(&captured);
        let replayed = Session::replay(log, &mut InertReducer)
            .await
            .expect("a seeded log replays");

        assert_eq!(
            replayed.snapshot().kv_root,
            live_root,
            "born-knowing KV state must survive replay identically"
        );
        assert_eq!(
            replayed.event_count(),
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
        async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
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
            .deliver(
                inbound(),
                None,
                &mut TimerByFamilyReducer,
                &authz,
                &mut exec,
            )
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
        let captured = attach_recording_sink(&mut s);
        s.deliver(
            inbound(),
            None,
            &mut NowReducer,
            &now_cap(),
            &mut StuckClock(1000),
        )
        .await
        .unwrap();
        assert!(
            s.event_count() > 1,
            "the session must carry post-genesis events so replay-stability is a NON-vacuous claim"
        );

        // (1) It IS log[0]'s Event::hash — the canonical durable head, not the reducer body hash. It stays
        // the genesis head even after folds appended later events (identity is anchored at log[0]).
        assert_eq!(
            s.genesis_hash(),
            s.genesis_ref().hash(),
            "genesis_hash must be the hash of the genesis EVENT (log[0]), not something else"
        );
        // (2) It is NOT the reducer hash — genesis_hash wraps the reducer in the genesis event framing, so
        // the two values differ (guards against a refactor that silently aliases them).
        assert_ne!(
            s.genesis_hash(),
            Hash::of(b"reducer-A"),
            "genesis_hash hashes the whole genesis event, so it must differ from the bare reducer hash"
        );
        // (3) STABLE across a REAL replay: reconstruct the session from its OWN persisted log (read back
        // from the durable source, not the resident Vec) via Session::replay (folding each event back
        // through the reducer) and assert the identity survives — the genuine recovery round-trip.
        let replayed = Session::replay(replay_input(&captured), &mut NowReducer)
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
    fn settled_set_watermark_advances_and_bounds_exceptions_d3() {
        // log-decouple I4 / D3: settled ≡ id < watermark || exceptions.contains(id). An IN-ORDER settle
        // advances the watermark with EMPTY exceptions (bounded); an OUT-OF-ORDER settle holds a sparse
        // exception until the gap fills, then collapses into the watermark. A late/duplicate settle for a
        // below-watermark id still reads settled (timeout-cancels §16c-S4).
        let mut s = SettledSet::default();
        assert!(!s.is_settled(0));
        // In-order: settling 0,1,2 advances the watermark to 3 with no lingering exceptions.
        s.insert(0);
        s.insert(1);
        s.insert(2);
        assert_eq!(s.watermark, 3);
        assert!(
            s.exceptions.is_empty(),
            "contiguous settles collapse into the watermark"
        );
        assert!(s.is_settled(0) && s.is_settled(1) && s.is_settled(2));
        assert!(!s.is_settled(3));
        // Out-of-order: settle 5 (gap at 3,4) → held as an exception, watermark unchanged.
        s.insert(5);
        assert_eq!(s.watermark, 3);
        assert_eq!(s.exceptions, [5].into_iter().collect());
        assert!(s.is_settled(5) && !s.is_settled(3) && !s.is_settled(4));
        // Fill the gap: settle 3 then 4 → the 3,4,5 run collapses, watermark jumps to 6, exceptions empty.
        s.insert(3);
        s.insert(4);
        assert_eq!(s.watermark, 6);
        assert!(
            s.exceptions.is_empty(),
            "filling the gap collapses the run into the watermark"
        );
        // A late/duplicate settle for a below-watermark id is a no-op + still reads settled.
        s.insert(1);
        assert_eq!(s.watermark, 6);
        assert!(s.is_settled(1));
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
        async fn perform(
            &mut self,
            _id: EffectId,
            req: &EffectRequest,
            _key: Hash,
        ) -> EffectOutcome {
            assert_eq!(req.kind, EffectKind::Http);
            EffectOutcome::err("PERMANENT: 400 bad request".to_string())
        }
    }

    // Emits an Http effect on inbound; on its result, records the Err reason into KV so the test can see the
    // failure reached the reducer as a normal folded event (§9d anti-stuck).
    struct HttpThenRecordReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for HttpThenRecordReducer {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
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
        let captured = attach_recording_sink(&mut s);
        s.deliver(
            EventBody::Inbound {
                content_type: ContentType {
                    family: "message".into(),
                    version: 1,
                },
                payload: Payload::Inline(b"go".to_vec().into()),
            },
            None,
            &mut HttpThenRecordReducer,
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
        // An Err EffectResult is on the DURABLE log (read back from the source — the failure record a
        // supervisor/replay sees is what was persisted, not an in-memory Vec).
        assert!(
            replay_input(&captured).iter().any(|e| matches!(
                &e.body,
                EventBody::EffectResult {
                    result: EffectOutcome::Err { .. },
                    ..
                }
            )),
            "the Err outcome is a first-class log event"
        );
    }

    // ---- userspace-effects I2: EffectOutcome::Deferred + settle_effect_result --------------------------
    //
    // A routed executor that DEFERS: it returns EffectOutcome::Deferred to say "I forwarded this for async
    // fulfillment, don't answer now" (models a UserspaceEffectExecutor delegating to a registered handler
    // session). The kernel must leave the Dispatched frame OPEN (no EffectResult) until a later
    // settle_effect_result(id, real-outcome) folds the answer back + resumes the emitting reducer.
    struct DeferringExecutor;
    #[async_trait::async_trait(?Send)]
    impl Executor for DeferringExecutor {
        async fn perform(
            &mut self,
            _id: EffectId,
            _req: &EffectRequest,
            _key: Hash,
        ) -> EffectOutcome {
            EffectOutcome::Deferred
        }
    }

    // Emits an Http effect on inbound; on its (eventually-settled) result, records the Ok bytes into KV so
    // the test can see the deferred answer reached the reducer's continuation as a normal folded event.
    struct HttpThenRecordOkReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for HttpThenRecordOkReducer {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => FoldOutput::with_effects(vec![Effect {
                    request: EffectRequest::new(
                        EffectKind::Http,
                        "https://ok.host/x",
                        None,
                        Timeliness::Interactive,
                    ),
                    token: Some(b"cont".to_vec()),
                }]),
                EventBody::EffectResult {
                    result: EffectOutcome::Ok(Some(Payload::Inline(bytes))),
                    ..
                } => {
                    kv.put(b"answer".to_vec(), bytes.to_vec());
                    FoldOutput::none()
                }
                _ => FoldOutput::none(),
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_deferred_effect_stays_open_until_settle_effect_result_folds_the_answer() {
        // userspace-effects I2: an executor returning Deferred leaves the effect OPEN (no EffectResult
        // folded); a later settle_effect_result folds the real answer + resumes the emitting reducer's
        // continuation. This is the general mechanism control/signature's fold-back is now a special case of.
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Http,
            predicate: ResourcePredicate::Any,
        }]);
        let mut exec = DeferringExecutor;
        let mut s = Session::genesis(Hash::of(b"deferred-v1"), Hash::of(b"nonce"));
        let captured = attach_recording_sink(&mut s);
        let inbound = || EventBody::Inbound {
            content_type: ContentType {
                family: "message".into(),
                version: 1,
            },
            payload: Payload::Inline(b"go".to_vec().into()),
        };
        s.deliver(
            inbound(),
            None,
            &mut HttpThenRecordOkReducer,
            &authz,
            &mut exec,
        )
        .await
        .unwrap();

        // The executor deferred → the effect is OPEN (Dispatched frame written, NO EffectResult folded yet),
        // and the reducer's continuation has NOT resumed.
        assert_eq!(
            s.open_effects(),
            1,
            "a deferred effect stays OPEN — the kernel wrote the Dispatched frame but recorded no result"
        );
        assert_eq!(
            s.kv().get(b"answer").as_deref(),
            None,
            "the continuation hasn't resumed — no answer yet"
        );
        assert!(
            !replay_input(&captured)
                .iter()
                .any(|e| matches!(e.body, EventBody::EffectResult { .. })),
            "no EffectResult is folded for a Deferred outcome (it's a transient signal, never logged)"
        );

        // Find the open effect's id (the Dispatched frame), then settle it off-band with the real answer.
        let id = replay_input(&captured)
            .iter()
            .find_map(|e| match &e.body {
                EventBody::Dispatched { id, .. } => Some(*id),
                _ => None,
            })
            .expect("a Dispatched frame was written for the deferred effect");
        let settled = s
            .settle_effect_result(
                id,
                EffectOutcome::Ok(Some(Payload::Inline(b"the-answer".to_vec().into()))),
                &mut HttpThenRecordOkReducer,
                &authz,
                &mut exec,
            )
            .await;
        assert!(settled, "settling an open deferred effect returns true");
        // The continuation resumed: the reducer folded the EffectResult + recorded the answer.
        assert_eq!(
            s.kv().get(b"answer").map(|v| v.to_vec()),
            Some(b"the-answer".to_vec()),
            "settle_effect_result folds the real answer back → the emitting reducer resumes with it"
        );
        assert_eq!(s.open_effects(), 0, "the settled effect is no longer open");

        // At-most-once: a duplicate settle is a no-op (no second EffectResult).
        let dup = s
            .settle_effect_result(
                id,
                EffectOutcome::Ok(Some(Payload::Inline(b"other".to_vec().into()))),
                &mut HttpThenRecordOkReducer,
                &authz,
                &mut exec,
            )
            .await;
        assert!(
            !dup,
            "a duplicate settle of an already-settled id is a no-op"
        );
        assert_eq!(
            replay_input(&captured)
                .iter()
                .filter(|e| matches!(e.body, EventBody::EffectResult { .. }))
                .count(),
            1,
            "exactly one EffectResult — the dup settle appended nothing",
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn status_snapshot_in_flight_is_derived_from_the_open_table_and_is_recovery_equivalent() {
        // Log-decouple I5 step 2: the status snapshot's in-flight list + event-count/last-kind are derived
        // from the resident open-obligation table (I2) + tip (I1), with ZERO log access — the seam that lets
        // step 3 drop the resident log Vec. Pin BOTH the derivation AND recovery-equivalence: a session
        // replayed from its log must rebuild an obligation table that yields the BYTE-IDENTICAL snapshot
        // (the deadline anchor + kind + target survive replay). Uses a Deferred effect so a Dispatched frame
        // stays OPEN (the only way an effect is genuinely in-flight).
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Http,
            predicate: ResourcePredicate::Any,
        }]);
        let mut exec = DeferringExecutor;
        let mut s = Session::genesis(Hash::of(b"in-flight-v1"), Hash::of(b"nonce"));
        let captured = attach_recording_sink(&mut s);
        let inbound = || EventBody::Inbound {
            content_type: ContentType {
                family: "message".into(),
                version: 1,
            },
            payload: Payload::Inline(b"go".to_vec().into()),
        };
        s.deliver(
            inbound(),
            None,
            &mut HttpThenRecordOkReducer,
            &authz,
            &mut exec,
        )
        .await
        .unwrap();

        // The deferred Http effect is OPEN → it surfaces in the DERIVED in-flight list (kind + target read
        // from the obligation, not a log scan), and the session is Active.
        let snap = s.status_snapshot(Some(500), 300_000);
        assert_eq!(snap.state, SessionState::Active);
        assert_eq!(snap.in_flight.len(), 1, "the open Http effect is in-flight");
        assert_eq!(
            snap.in_flight[0].schema_hash,
            Some(crate::ast_marshal::builtin_effect_schema_hash(
                &EffectKind::Http
            ))
        );
        assert_eq!(snap.in_flight[0].target, "https://ok.host/x");
        assert_eq!(
            snap.armed_timers, 0,
            "no timers — a real effect, not a timer"
        );
        // event_count/last_event_kind are derived off the resident tip: Genesis, Inbound, Dispatched = 3.
        assert_eq!(snap.event_count, 3);
        assert_eq!(snap.last_event_kind, "Dispatched");

        // RECOVERY-EQUIVALENCE: replay the exact log → the rebuilt obligation table must yield the SAME
        // derived snapshot (this is what makes the eventual Vec-drop safe — recovery re-derives, not re-reads).
        let replayed = Session::replay(replay_input(&captured), &mut HttpThenRecordOkReducer)
            .await
            .expect("replay");
        let rsnap = replayed.status_snapshot(Some(500), 300_000);
        assert_eq!(rsnap.state, snap.state, "replayed state matches live");
        assert_eq!(rsnap.event_count, snap.event_count);
        assert_eq!(rsnap.last_event_kind, snap.last_event_kind);
        assert_eq!(rsnap.armed_timers, snap.armed_timers);
        assert_eq!(rsnap.in_flight.len(), snap.in_flight.len());
        assert_eq!(
            rsnap.in_flight[0].schema_hash,
            snap.in_flight[0].schema_hash
        );
        assert_eq!(
            rsnap.in_flight[0].target, snap.in_flight[0].target,
            "the in-flight target is rebuilt identically on replay"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn settle_effect_result_rejects_a_deferred_outcome_and_unknown_ids() {
        // Guards: settling WITH Deferred is nonsensical (a "no result yet" signal, not a result) → no-op, so
        // the effect can't be left open-but-"settled". Settling an unknown/never-dispatched id → no-op too.
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Http,
            predicate: ResourcePredicate::Any,
        }]);
        let mut exec = DeferringExecutor;
        let mut s = Session::genesis(Hash::of(b"deferred-guard-v1"), Hash::of(b"nonce"));
        let captured = attach_recording_sink(&mut s);
        s.deliver(
            EventBody::Inbound {
                content_type: ContentType {
                    family: "message".into(),
                    version: 1,
                },
                payload: Payload::Inline(b"go".to_vec().into()),
            },
            None,
            &mut HttpThenRecordOkReducer,
            &authz,
            &mut exec,
        )
        .await
        .unwrap();
        let id = replay_input(&captured)
            .iter()
            .find_map(|e| match &e.body {
                EventBody::Dispatched { id, .. } => Some(*id),
                _ => None,
            })
            .expect("dispatched");
        // Settling WITH Deferred → no-op, effect STAYS open (not falsely settled).
        assert!(
            !s.settle_effect_result(
                id,
                EffectOutcome::Deferred,
                &mut HttpThenRecordOkReducer,
                &authz,
                &mut exec
            )
            .await,
            "settling with Deferred is a no-op (Deferred is not a real outcome)"
        );
        assert_eq!(
            s.open_effects(),
            1,
            "a Deferred-settle left the effect OPEN, not falsely settled"
        );
        // Unknown id → no-op.
        assert!(
            !s.settle_effect_result(
                EffectId(9999),
                EffectOutcome::Ok(None),
                &mut HttpThenRecordOkReducer,
                &authz,
                &mut exec
            )
            .await,
            "settling an unknown/never-dispatched id is a no-op"
        );
    }

    // §6 terminal-tip completeness: settle_effect_result AND time_out_effect must refuse a CLOSED session
    // (like deliver/fire_due_timers) — a self-close can leave an effect in-flight, and settling or timing out
    // its late result would append an EffectResult PAST the terminal Closed event, un-tailing it. This
    // completes the is_closed guard across EVERY append path (github-liaison #2381 guard-every-entry-point).
    struct SelfCloseOnInbound;
    #[async_trait::async_trait(?Send)]
    impl Reducer for SelfCloseOnInbound {
        async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    FoldOutput::close(crate::event::CloseOutcome::Success(
                        crate::effect::Payload::Inline(Vec::new().into()),
                    ))
                }
                _ => FoldOutput::none(),
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_closed_session_settles_and_times_out_no_in_flight_effect() {
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Http,
            predicate: ResourcePredicate::Any,
        }]);
        let mut exec = DeferringExecutor;
        let mut s = Session::genesis(Hash::of(b"close-settle-v1"), Hash::of(b"nonce"));
        let captured = attach_recording_sink(&mut s);
        // Dispatch an Http effect that stays OPEN (Deferred), then self-close with it in-flight.
        s.deliver(
            inbound(),
            None,
            &mut HttpThenRecordOkReducer,
            &authz,
            &mut exec,
        )
        .await
        .unwrap();
        let id = replay_input(&captured)
            .iter()
            .find_map(|e| match &e.body {
                EventBody::Dispatched { id, .. } => Some(*id),
                _ => None,
            })
            .expect("http dispatched + open");
        s.deliver(
            inbound(),
            None,
            &mut SelfCloseOnInbound,
            &Authorizer::deny_all(),
            &mut exec,
        )
        .await
        .unwrap();
        assert!(
            s.is_closed(),
            "self-closed with the Http effect still in-flight"
        );
        let count_at_close = s.event_count();

        // Settling the in-flight effect's late result is REFUSED — no EffectResult past the terminal Closed.
        assert!(
            !s.settle_effect_result(
                id,
                EffectOutcome::Ok(None),
                &mut HttpThenRecordOkReducer,
                &authz,
                &mut exec
            )
            .await,
            "a closed session settles no in-flight effect result"
        );
        // Timing it out is likewise REFUSED.
        assert!(
            !s.time_out_effect(id, &mut HttpThenRecordOkReducer, &authz, &mut exec)
                .await,
            "a closed session times out no in-flight effect"
        );
        assert_eq!(
            s.event_count(),
            count_at_close,
            "no event was appended past the terminal Closed"
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
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
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
                    // CARRY THE PRIOR SUMMARY FORWARD: a self-hosting agent compacts REPEATEDLY as context
                    // re-saturates, so the fold must SEED from the existing summary/latest (if any) and append
                    // the new detail — otherwise each cycle would overwrite + LOSE every earlier cycle's
                    // summarized content. (A real summary is a model fold; here concatenation stands in — the
                    // load-bearing property is that no earlier content is dropped across cycles.)
                    let mut summary: Vec<u8> = kv
                        .get(b"summary/latest")
                        .map(|s| s.to_vec())
                        .unwrap_or_default();
                    let mut keys: Vec<Vec<u8>> = Vec::new();
                    for i in 0u64.. {
                        let k = format!("detail/{i}").into_bytes();
                        match kv.get(&k) {
                            Some(v) => {
                                if !summary.is_empty() {
                                    summary.push(b'|');
                                }
                                summary.extend_from_slice(&v);
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
        let captured = attach_recording_sink(&mut s);
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
            s.deliver(feed(msg), None, &mut CompactingReducer, &authz, &mut exec)
                .await
                .expect("deliver detail");
        }
        assert_eq!(s.kv().get(b"detail/0").as_deref(), Some(&b"alpha"[..]));
        assert_eq!(s.kv().get(b"detail/2").as_deref(), Some(&b"gamma"[..]));
        assert_eq!(
            s.kv().len(),
            3,
            "three detail entries accumulated in the working set"
        );

        // A compact turn: fold detail/* → summary/latest + prune the detail keys.
        s.deliver(
            feed(b"compact"),
            None,
            &mut CompactingReducer,
            &authz,
            &mut exec,
        )
        .await
        .expect("deliver compact");

        // The working set is now BOUNDED — one summary entry, all detail pruned.
        assert_eq!(
            s.kv().get(b"summary/latest").as_deref(),
            Some(&b"alpha|beta|gamma"[..]),
            "compaction folds every detail entry into one summary"
        );
        assert_eq!(
            s.kv().get(b"detail/0").as_deref(),
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

        // The pattern is REPLAY-DETERMINISTIC (no kernel change): a fresh replay of the log read back from
        // the durable source reconstructs the identical post-compaction kv (compaction is an ordinary fold,
        // already on the log).
        let replayed = Session::replay(replay_input(&captured), &mut CompactingReducer)
            .await
            .expect("replay a compacted session");
        assert_eq!(
            replayed.kv().get(b"summary/latest").as_deref(),
            Some(&b"alpha|beta|gamma"[..])
        );
        assert_eq!(replayed.kv().get(b"detail/0"), None);
        assert_eq!(
            replayed.kv().len(),
            1,
            "replay reconstructs the identical bounded working set — compaction is just a fold on the log"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn gap4_option_a_compaction_carries_the_prior_summary_across_repeated_cycles() {
        // A self-hosting agent compacts REPEATEDLY as context re-saturates — so the fold must CARRY the prior
        // summary forward, not overwrite it. This pins that no earlier cycle's summarized content is lost:
        // detail turns → compact → more detail → compact again → the summary holds BOTH cycles' content, and
        // the working set stays bounded (one summary) across cycles. (Without seeding from summary/latest,
        // the second compact would drop cycle 1 — the fidelity gap this guards.)
        let mut exec = RecordingExecutor::new();
        let mut s = Session::genesis(Hash::of(b"gap4-multicycle-v1"), Hash::of(b"nonce"));
        let captured = attach_recording_sink(&mut s);
        let authz = Authorizer::deny_all();
        let feed = |body: &[u8]| EventBody::Inbound {
            content_type: ContentType {
                family: "message".into(),
                version: 1,
            },
            payload: crate::effect::Payload::Inline(body.to_vec().into()),
        };

        // Cycle 1: two details → compact.
        for msg in [b"detail:a1".as_slice(), b"detail:a2"] {
            s.deliver(feed(msg), None, &mut CompactingReducer, &authz, &mut exec)
                .await
                .expect("c1 detail");
        }
        s.deliver(
            feed(b"compact"),
            None,
            &mut CompactingReducer,
            &authz,
            &mut exec,
        )
        .await
        .expect("c1 compact");
        assert_eq!(
            s.kv().get(b"summary/latest").as_deref(),
            Some(&b"a1|a2"[..])
        );
        assert_eq!(s.kv().len(), 1, "bounded after cycle 1");

        // Cycle 2: two more details → compact. The prior summary (a1|a2) must be CARRIED, not overwritten.
        for msg in [b"detail:b1".as_slice(), b"detail:b2"] {
            s.deliver(feed(msg), None, &mut CompactingReducer, &authz, &mut exec)
                .await
                .expect("c2 detail");
        }
        s.deliver(
            feed(b"compact"),
            None,
            &mut CompactingReducer,
            &authz,
            &mut exec,
        )
        .await
        .expect("c2 compact");
        assert_eq!(
            s.kv().get(b"summary/latest").as_deref(),
            Some(&b"a1|a2|b1|b2"[..]),
            "the second compaction carries cycle 1's summary forward — no earlier content is lost"
        );
        assert_eq!(
            s.kv().len(),
            1,
            "the working set stays bounded (one summary) across repeated compaction cycles"
        );

        // Still replay-deterministic across multiple compaction cycles (log read back from the source).
        let replayed = Session::replay(replay_input(&captured), &mut CompactingReducer)
            .await
            .expect("replay a multi-cycle-compacted session");
        assert_eq!(
            replayed.kv().get(b"summary/latest").as_deref(),
            Some(&b"a1|a2|b1|b2"[..])
        );
        assert_eq!(replayed.kv().len(), 1);
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
    use crate::test_log_source::*;

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
        async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
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
            &mut TimerArmingReducer,
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
        let len_after_marker = s.event_count();

        // The next delivery is REFUSED — not applied, not silently dropped.
        let refused = s
            .deliver(
                inbound(),
                None,
                &mut TimerArmingReducer,
                &timer_cap(),
                &mut RecordingExecutor::new(),
            )
            .await;
        assert!(
            matches!(refused, Err(KernelError::FoldRefused)),
            "a fold on a terminated session must return FoldRefused, got {refused:?}"
        );
        assert_eq!(
            s.event_count(),
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
        let captured = attach_recording_sink(&mut s);
        s.deliver(
            inbound(),
            None,
            &mut TimerArmingReducer,
            &timer_cap(),
            &mut RecordingExecutor::new(),
        )
        .await
        .unwrap();
        s.append(terminated_marker(), None).await;

        let recovered = Session::replay(replay_input(&captured), &mut TimerArmingReducer)
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
                &mut TimerArmingReducer,
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
            &mut TimerArmingReducer,
            &timer_cap(),
            &mut RecordingExecutor::new(),
        )
        .await
        .unwrap();
        assert_eq!(s.next_timer_deadline(), Some(1000), "a timer is armed");

        s.append(terminated_marker(), None).await;
        assert!(s.is_terminated());
        let len_after_marker = s.event_count();

        let fired = s
            .fire_due_timers(
                1500,
                &mut TimerArmingReducer,
                &timer_cap(),
                &mut RecordingExecutor::new(),
            )
            .await;
        assert_eq!(fired, 0, "a terminated session fires no timers");
        assert_eq!(
            s.event_count(),
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
            &mut TimerArmingReducer,
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
        let len_after_marker = s.event_count();

        // time_out_effect on the terminated session is a no-op — no EffectResult appended, tail preserved.
        let timed_out = s
            .time_out_effect(
                open_id,
                &mut TimerArmingReducer,
                &timer_cap(),
                &mut RecordingExecutor::new(),
            )
            .await;
        assert!(!timed_out, "a terminated session times out no effect");
        assert_eq!(
            s.event_count(),
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
            &mut TimerArmingReducer,
            &timer_cap(),
            &mut RecordingExecutor::new(),
        )
        .await
        .unwrap();
        let len_before = s.event_count();
        assert!(!s.is_terminated());
        // Capture the prior tip so we can ASSERT the marker's causal edge points at it (not just claim it).
        let prior_tip = s.tip().hash();

        // terminate() appends the marker (log grows by exactly 1) + returns its hash; the reducer did NOT
        // run on it (fold-free) — the marker is the tail, cause-linked to the prior tip.
        let by = Hash::of(b"controller-session");
        let marker_hash = s
            .terminate(by, "operator kill".to_string())
            .await
            .expect("terminate on a live session succeeds");
        assert_eq!(
            s.event_count(),
            len_before + 1,
            "terminate appends exactly one event"
        );
        assert!(s.is_terminated(), "the session is now terminated");
        let tail = s.tip();
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
                &mut TimerArmingReducer,
                &timer_cap(),
                &mut RecordingExecutor::new(),
            )
            .await;
        assert!(matches!(refused, Err(KernelError::FoldRefused)));

        // A SECOND terminate is rejected — no double-marker (idempotent-by-rejection), log unchanged.
        let len_after = s.event_count();
        let second = s.terminate(by, "again".to_string()).await;
        assert!(
            matches!(second, Err(KernelError::FoldRefused)),
            "terminating an already-terminated session returns FoldRefused, got {second:?}"
        );
        assert_eq!(
            s.event_count(),
            len_after,
            "a rejected second terminate appends nothing"
        );
    }

    // §lifecycle I2 (Spawned edge): record_spawn appends a parent→child edge fold-free + spawned_children
    // reads them back in order; the edge is cause-linked + replay-stable; a terminated parent can't spawn.
    #[tokio::test(flavor = "current_thread")]
    async fn record_spawn_appends_parent_child_edges_readable_and_replay_stable() {
        let mut parent = Session::genesis(Hash::of(b"parent-reducer"), Hash::of(b"parent-nonce"));
        let captured = attach_recording_sink(&mut parent);
        parent
            .deliver(
                inbound(),
                None,
                &mut TimerArmingReducer,
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
        let len_before = parent.event_count();
        let prior_tip = parent.tip().hash();
        let edge_a = parent.record_spawn(child_a).await.expect("record child A");
        assert_eq!(
            parent.event_count(),
            len_before + 1,
            "record_spawn appends exactly one event"
        );
        let tail_a = parent.tip();
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
        let replayed = Session::replay(replay_input(&captured), &mut TimerArmingReducer)
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
    use crate::test_log_source::*;

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
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
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

        s.deliver(inbound(), None, &mut SetThenResolve, &AllowStore, &mut exec)
            .await
            .unwrap();
        // (AllowStore isolates the ARM here; production grantability via Capability::for_family is proven
        // in `store_effects_are_grantable_via_capability_for_family` below and in authz.rs's family-grant test.)

        // The reducer set then resolved system/compiler/latest; the resolved hash it recorded must equal
        // the hash it set — the store round-tripped THROUGH the kernel's store arm (set applied, resolve
        // read the latest). And the executor NEVER saw a store effect (it's not executor-routed).
        assert_eq!(
            s.kv().get(b"resolved").as_deref(),
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
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
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
        let mut reducer = JoinThenResolveAll {
            group: "session/room/lobby",
            m1: Hash::of(b"member-A"),
            m2: Hash::of(b"member-B"),
            origin: Hash::of(b"origin-session"),
        };

        s.deliver(inbound(), None, &mut reducer, &AllowStore, &mut exec)
            .await
            .unwrap();

        // The reducer added 2 members then resolve-all'd → the decoded membership it recorded is exactly 2
        // (the group round-tripped THROUGH the kernel's group arm: adds applied, resolve-all folded add-wins).
        assert_eq!(
            s.kv().get(b"member_count").as_deref(),
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
        async fn perform(
            &mut self,
            _id: EffectId,
            req: &EffectRequest,
            _key: Hash,
        ) -> EffectOutcome {
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
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
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
        let mut reducer = AgentLoopReducer {
            model: "anthropic.claude",
        };

        s.deliver(inbound(), None, &mut reducer, &AllowAllAuthz, &mut exec)
            .await
            .unwrap();

        // The loop ran to end_turn and recorded the final answer.
        assert_eq!(
            s.kv().get(b"answer").as_deref(),
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
        s.deliver(inbound(), None, &mut SetThenResolve, &authz, &mut exec)
            .await
            .unwrap();
        assert_eq!(
            s.kv().get(b"resolved").as_deref(),
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
            .deliver(
                inbound(),
                None,
                &mut SetThenResolve,
                &AllowStore,
                &mut exec_a,
            )
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
            .deliver(inbound(), None, &mut ResolveOnly, &AllowStore, &mut exec_b)
            .await
            .unwrap();
        assert_eq!(
            consumer.kv().get(b"resolved").as_deref(),
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
            .deliver(
                inbound(),
                None,
                &mut SetThenResolve,
                &AllowStore,
                &mut exec_a,
            )
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
            .deliver(inbound(), None, &mut ResolveOnly, &AllowStore, &mut exec_b)
            .await
            .unwrap();
        assert_eq!(
            consumer.kv().get(b"resolved").as_deref(),
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
        s.deliver(inbound(), None, &mut SetThenResolve, &AllowStore, &mut exec)
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
            &mut SetThenResolve,
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
        async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
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
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
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
            &mut MismatchedSetReducer,
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
            .deliver(
                inbound(),
                None,
                &mut ResolveOnly,
                &resolve_only(),
                &mut exec,
            )
            .await
            .unwrap();
        assert_eq!(
            reader.kv().get(b"resolved").as_deref(),
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
                &mut SetThenResolve,
                &resolve_only(),
                &mut exec2,
            )
            .await
            .unwrap();
        assert_eq!(
            writer.kv().get(b"resolved").as_deref(),
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
        async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
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
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
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
        let mut publish = PublishName {
            name: "session/alice".to_string(),
            hash: alice_id,
        };
        publisher
            .deliver(
                inbound(),
                None,
                &mut publish,
                &session_store_cap(),
                &mut exec_b,
            )
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
                &mut ResolveName {
                    name: "session/alice".to_string(),
                },
                &session_store_cap(),
                &mut exec_a,
            )
            .await
            .unwrap();
        assert_eq!(
            resolver.kv().get(b"resolved").as_deref(),
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
        let mut publish = PublishName {
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
        s.deliver(inbound(), None, &mut publish, &wrong_cap, &mut exec)
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
        async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
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
        let captured = attach_recording_sink(&mut s);
        s.deliver(
            inbound(),
            None,
            &mut ShellPipelineEmitReducer {
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
            !replay_input(&captured)
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
        let captured = attach_recording_sink(&mut s);
        s.deliver(
            inbound(),
            None,
            &mut ShellPipelineEmitReducer {
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
            replay_input(&captured)
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
    // v-ah-host: blob/put → Ok(Inline(hash raw bytes)); blob/get(raw-bytes target) → Ok(Some(Inline(bytes)))/Ok(None).

    /// A STUB content-addressed store executor: blob/put stores bytes keyed by content hash + returns the RAW
    /// hash bytes (v-ah-host's BlobExecutor convention — no hex); blob/get(raw-bytes target) returns the stored
    /// bytes (or Ok(None) if absent). Stands in for the real host BlobExecutor so the reducer FOLD is provable
    /// in-kernel.
    struct StubBlobExecutor {
        blobs: std::collections::HashMap<Vec<u8>, Vec<u8>>,
    }
    #[async_trait::async_trait(?Send)]
    impl Executor for StubBlobExecutor {
        async fn perform(
            &mut self,
            _id: EffectId,
            req: &EffectRequest,
            _key: Hash,
        ) -> EffectOutcome {
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
                    let key = Hash::of(&bytes).as_bytes().to_vec();
                    self.blobs.insert(key.clone(), bytes);
                    EffectOutcome::Ok(Some(Payload::Inline(key.into())))
                }
                effect_ct::BLOB_GET => {
                    // Target = the RAW 32 hash bytes (the handle blob/put returned + the reducer store/resolve'd)
                    // — read from the opaque byte target directly, no hex (the runtime no-hex directive).
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
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
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
                    // 1) blob/put result = the raw hash bytes → store/set the doc name at it.
                    // 2) store/resolve result = a name-set payload (name, hash) → blob/get the hash.
                    // 3) blob/get result = the doc-AST bytes → record recovered.
                    if kv.get(b"published").is_none() {
                        // Phase 1: blob/put returned the raw hash bytes. Register it in the doc index.
                        kv.put(b"published".to_vec(), bytes.to_vec()); // remember the raw hash bytes
                        let hash = match <[u8; 32]>::try_from(bytes.as_ref()) {
                            Ok(a) => Hash::from_bytes(a),
                            Err(_) => return FoldOutput::none(),
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
                            h.as_bytes(), // the raw hash bytes are the blob/get target
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
        let mut reducer = DocPublishReducer { name };
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
        s.deliver(publish, None, &mut reducer, &AllowStoreAndBlob, &mut exec)
            .await
            .expect("publish delivers");

        // The doc index now points memory/doc/<pkg> at the content hash (registered via store/set).
        let published_bytes = s
            .kv()
            .get(b"published")
            .expect("published the raw hash bytes")
            .to_vec();
        assert_eq!(
            published_bytes,
            Hash::of(&doc_ast).as_bytes().to_vec(),
            "the doc-index registered the content hash (raw bytes) of the published doc-AST"
        );

        // QUERY: deliver a doc/query inbound → resolve the name → blob/get → recover the doc-AST.
        let query = EventBody::Inbound {
            content_type: ContentType {
                family: "doc/query".into(),
                version: 1,
            },
            payload: Payload::Inline(b"".to_vec().into()),
        };
        s.deliver(query, None, &mut reducer, &AllowStoreAndBlob, &mut exec)
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
