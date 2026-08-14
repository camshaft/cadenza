//! Events and the per-session log.
//!
//! The log is the append-only, ordered record of everything that happened in a session. It IS the
//! state (§14a) — KV is a derived projection, snapshots are checkpoints of it. Every event is wrapped
//! in a thin **envelope** carrying the fields the review said must exist from day one: `cause` (the
//! causal parent, for the DAG §5), a `content_type` tag the kernel routes on but never interprets
//! (§9b), and — later — a signature + producer identity (§10; carried as optional now, unverified in
//! v0). The kernel treats the payload as opaque; only reducers/executors interpret it.

use crate::effect::{EffectId, Payload};
use crate::hash::Hash;

/// Position of an event within a session log: a dense 0-based index. Total order within a session
/// (§3); there is no global order across sessions (§4b).
pub type SeqNo = u64;

/// A structured content-type tag (§9b): `family` + `version`, so tolerant readers can match on family
/// and range-check version. The kernel carries it opaquely for routing/filtering; it is a HINT, never
/// a trusted type assertion (§9b boundary).
///
/// `family` is a `Cow<'static, str>` (operator Bytes/cheap-clone directive): the well-known effect families
/// are `&'static str` consts ([`crate::effect::effect_ct`]), so a content-type built from a kind holds
/// `Cow::Borrowed` — ZERO heap allocation on the hot effect path (an effect's `content_type.family` was a
/// fresh `String` per `EffectRequest::new` before this). A runtime-derived family (a decoded/inbound one)
/// holds `Cow::Owned`; both compare + deref to `&str` identically, so read/deref/compare callers are
/// unaffected — only a caller that ASSIGNS a `String` to `family` now needs `.into()` (`String` and
/// `&'static str` both `Into<Cow>`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ContentType {
    pub family: std::borrow::Cow<'static, str>,
    pub version: u32,
}

impl ContentType {
    /// The well-known **"report" content-type** (v1) — the fork-for-query summarize protocol contract
    /// (operator ruling (a), fork-for-query design). An ephemeral query fork ([`crate::kernel::Session::fork_for_query`])
    /// delivers an `Inbound` carrying THIS content-type; the reducer recognizes it (via [`ContentType::is_report`])
    /// and describes its own state — from local KV/goal/progress where it can (no model call, the operator's
    /// preference), or via a scoped model call where it must. The kernel never interprets it (§9b: content-type
    /// is a routing HINT, not a trusted assertion); it's the AGREED family string reducers key off, so a debug
    /// query means the same thing to every reducer.
    pub const REPORT_FAMILY: &'static str = "report";

    /// Construct the well-known report content-type (`{family: "report", version: 1}`) — see
    /// [`ContentType::REPORT_FAMILY`]. Use this to build the summarize-query message a fork is delivered.
    pub fn report() -> Self {
        ContentType {
            // A `&'static str` const → `Cow::Borrowed`, zero-alloc.
            family: std::borrow::Cow::Borrowed(Self::REPORT_FAMILY),
            version: 1,
        }
    }

    /// Is this the fork-for-query "report" family? Matches on `family` ONLY (version-tolerant per §9b —
    /// a reducer accepts any report version it understands, range-checking `version` itself if it cares),
    /// so a reducer's fold can branch a summarize-query cheaply: `if event_ct.is_report() { …describe self… }`.
    pub fn is_report(&self) -> bool {
        self.matches_family(Self::REPORT_FAMILY)
    }

    /// The tolerant-reader family match (§9b, design §"content-type"): does this content-type belong to
    /// `family`, IGNORING version? This is the primitive a reducer/router keys off — match the family,
    /// then range-check the version separately with [`ContentType::version_in`] if it cares. Keeping the
    /// two checks distinct is exactly the "known family, unknown version → defer/reject honestly" behavior
    /// the design calls for (a v1 reader must not decode a `family/v2` payload as garbage). It is also the
    /// ONE place the family comparison lives: all family-keyed matching (routing, authz, [`ContentType::is_report`])
    /// should go through this helper rather than compare `self.family` inline, so the check can't drift across sites.
    pub fn matches_family(&self, family: &str) -> bool {
        self.family == family
    }

    /// The version half of the tolerant-reader check: is `version` within the INCLUSIVE `[min, max]` range
    /// this reader understands? Pair with [`ContentType::matches_family`] — `matches_family(f) &&
    /// version_in(1, 3)` is "a known `f`, at a version I can handle." A `family` match with a version
    /// OUTSIDE the range is the "known family, unknown version" case the reader defers/rejects rather than
    /// misdecoding (§9b / design §content-type). Total: an empty range (`min > max`) is simply never in.
    pub fn version_in(&self, min: u32, max: u32) -> bool {
        min <= self.version && self.version <= max
    }
}

/// The event body — what actually happened. This is the v0 vocabulary; it grows as features land.
/// Crucially it distinguishes the three obligation-bearing kinds the review (S1) said must be durable
/// LOG events, not ephemeral kernel metadata: `Dispatched`, `EffectResult`, and `TimerArmed`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum EventBody {
    /// The session's first event (§3 genesis): names the reducer to fold with (by content hash), a
    /// caller-supplied per-spawn `spawn_nonce` (entropy), and optional `parent` provenance.
    ///
    /// `spawn_nonce` (§lifecycle I2 / operator "hash of spawn-time + entropy" ruling): the kernel is
    /// clock-free + entropy-free (§9c), so the HOST mints this at spawn and passes it in. It's a `Hash`
    /// (blake3 content hash), so the host DERIVES it as `Hash::of(<spawn-unique bytes>)` — e.g.
    /// `Hash::of(&getrandom_bytes)` (an OS-entropy draw, not wall-clock, to avoid same-ms burst collisions)
    /// — NOT `Hash::from_bytes(random)` (which would fabricate a "hash" of nothing, violating Hash
    /// semantics). It lives in the durable seq-0 event, so it's replay-deterministic (recovery reads
    /// it from the log, NEVER re-mints — a re-mint would change the genesis hash = a different SessionId on
    /// recovery = corruption). It is what makes `genesis_hash()` per-SESSION unique: without it, two
    /// sessions over the same reducer produced an identical Genesis event → identical SessionId → registry
    /// collision (the gap v-agent-harness-host pinned).
    ///
    /// `parent` (§6 supervision / lifecycle I2): `Some(<parent's genesis hash>)` for a session SPAWNED by
    /// another (via `lifecycle/spawn`), `None` for a root/top-level session. Baking the parent into the
    /// hashed seq-0 body makes the child's id self-certify its provenance (the child genesis-hash is
    /// provenance-dependent); the durable `Spawned{child_hash}` edge in the PARENT's log is the other half
    /// of the same relation (I6's descendant-authority + §8's cascade walk it).
    Genesis {
        reducer: Hash,
        spawn_nonce: Hash,
        parent: Option<Hash>,
    },

    /// An inbound message delivered into this session (from a peer via `Emit`, from a broker/ingress,
    /// or from the operator). The reducer folds it. Opaque payload.
    Inbound {
        content_type: ContentType,
        payload: Payload,
    },

    /// **DURABLE dispatch record (§16c-S1).** Written to the authoritative log BEFORE the effect is
    /// routed to an executor. This is what makes crash recovery correct: on restart, a `Dispatched`
    /// with no matching `EffectResult` is a known in-flight obligation to re-drive (idempotently) or
    /// fail — never silently double-fire or drop.
    Dispatched {
        id: EffectId,
        /// The resolved target argument (url / session-id / command / hash — opaque bytes). `Arc<[u8]>`
        /// (operator Target=Bytes ruling 2026-08-09): the source [`crate::effect::EffectRequest::target`] is
        /// `Arc<[u8]>`, so recording it here is an O(1) refcount bump (`req.target.clone()`), and the durable
        /// frame carries the exact bytes the effect was dispatched with (a non-UTF-8 target is preserved
        /// faithfully on the log). On the wire it is length-prefixed bytes exactly as before — byte-identical
        /// to the old `Arc<str>` encoding (a str was already encoded as its UTF-8 bytes), so this is NOT a
        /// wire break. A reader wanting text uses a fail-closed UTF-8 view.
        target: std::sync::Arc<[u8]>,
        /// Idempotency key (§16c-S1/D): re-driving a dispatch with the same key must not double-apply.
        /// For naturally-idempotent effects this can equal the id; for side-effecting ones the
        /// executor dedups on it.
        idempotency_key: Hash,
        /// Absolute deadline for the auto-timeout (§9d), as a wall-clock ms anchor (§16c-S5) so it
        /// survives failover/migration — the reducer still never reads the clock.
        deadline_ms: Option<u64>,
        /// The reducer's OWN opaque continuation token for this effect, if it supplied one (§19e). A WASM
        /// `ComponentReducer` correlates continuations by this token, never by the kernel `EffectId`
        /// (which stays kernel-internal). Recording it HERE — in the durable Dispatched frame — is the
        /// §19e hard guard: the `EffectId ↔ token` bridge map is session state that MUST rebuild from the
        /// LOG on recovery, so the token can't live only in a volatile map. `None` for effects from a
        /// reducer that doesn't use a token (e.g. the in-process Rust `Reducer` trait), which is every
        /// dispatch until the wasm-reducer bridge (§19e slice 2) populates it.
        token: Option<Vec<u8>>,
        /// The dispatched effect's SCHEMA-HASH identity (schema-hash-ONLY effect model, slice-2 wire flip):
        /// the durable-frame mirror of [`crate::effect::EffectRequest::schema_hash`], copied straight from the
        /// effect at dispatch. This is now the SOLE identity of a dispatched effect on the frame — the legacy
        /// `kind: EffectKind` enum and `family: Arc<str>` string were DROPPED (schema-hash-only ruling: routing,
        /// authz, and recovery classification all key on this hash, never on a name/enum). MANDATORY (not
        /// `Option`): every dispatchable effect has a schema-hash — a built-in kind via
        /// [`crate::ast_marshal::builtin_effect_schema_hash`], a well-known non-kind family via
        /// `family_effect_schema_hash`. `None` for a register-by-string EXTENSION family (a userspace
        /// `effect/<name>` a handler serves): its identity is the producer-baked reify hash, but phase-1a
        /// reify emits the kind as a STRING on the INPUT wire (no hash yet), so `parse_effect_request` has no
        /// hash to record — those route by `content_type.family` on the input wire (phase-3, unchanged), not
        /// by this frame identity. MANDATORY schema_hash rides phase-3 (the input-wire kind→hash flip); until
        /// then this stays `Option<Hash>` — the legacy `kind` enum + `family` string are DROPPED regardless
        /// (schema-hash-only frame identity), so a well-known family carries `Some(hash)` and only an
        /// as-yet-schemaless extension is `None`. The `event_ast` wire-codec (absence-tolerant) is
        /// v-compiler-ml's 2c lane, folded into this same commit.
        schema_hash: Option<Hash>,
    },

    /// The result of a previously-`Dispatched` effect, correlated by `id` (§16c-S4). The reducer
    /// resumes its continuation for that id.
    EffectResult {
        id: EffectId,
        result: EffectOutcome,
        /// The reducer's continuation token for this effect (§19b/§19e (B)), COPIED from `id`'s
        /// `Dispatched` frame when the result is recorded. It rides the result event so a WASM
        /// `ComponentReducer`'s `fold` reads it back as the guest's `resumes` — without `fold` ever
        /// touching the log/map (fold stays a pure function of `(event, kv)`). `None` = the dispatch
        /// carried no token (a Rust reducer that correlates by `EffectId`). Derived from the durable
        /// Dispatched frame, so it's the same live and on replay.
        token: Option<Vec<u8>>,
    },

    /// A timer was armed. Durable (§16c-S1/S5) with an ABSOLUTE deadline so any node can compute the
    /// remaining time after failover. The kernel injects a `TimerFired` at the deadline.
    TimerArmed {
        id: EffectId,
        deadline_ms: u64,
        /// The reducer's OWN opaque continuation token for the timer effect (§19e), recorded here in the
        /// durable arming frame — the timer analogue of `Dispatched.token`. When the timer fires, the
        /// kernel copies this onto the `TimerFired` event so a WASM `ComponentReducer` reads it back as
        /// the guest's `resumes` (slice 2b-iii). `None` = a token-free reducer (the in-process Rust
        /// `Reducer`, which correlates by `EffectId`). Recording it in the durable frame (not a volatile
        /// map) is the same §19e recovery guard as `Dispatched.token`.
        token: Option<Vec<u8>>,
    },

    /// A timer fired (or an effect deadline elapsed). Carries the recorded fire time so replay reads a
    /// frozen fact and never consults a clock (§9c).
    TimerFired {
        id: EffectId,
        fired_ms: u64,
        /// The reducer's continuation token for the timer (§19b/§19e (B)), COPIED from `id`'s
        /// `TimerArmed` frame when the fire is recorded — the timer analogue of `EffectResult.token`. It
        /// rides the fire event so a WASM `ComponentReducer`'s `fold` reads it back as the guest's
        /// `resumes` without touching the log/map (fold stays pure). `None` = the timer was armed
        /// token-free. Derived from the durable arming frame, so it's the same live and on replay.
        token: Option<Vec<u8>>,
    },

    /// An authorization decision the kernel made about a requested effect — logged so an audit can
    /// replay not just what happened but whether it was permitted (§10). `denied` requests never reach
    /// an executor.
    AuthzDenied {
        id: EffectId,
        reason: String,
        /// The reducer's continuation token for the DENIED effect (§19b/§19e (B)), moved from the
        /// effect request the reducer emitted. A denial is a terminal outcome the reducer resumes on
        /// (`resumes_effect` recognizes it), so the token rides the denial event exactly as it rides an
        /// `EffectResult` — the guest's `fold` reads it back as `resumes` to run its denial branch.
        /// Unlike a dispatch/timer, there is NO prior durable frame to copy from (a denial is recorded
        /// BEFORE any `Dispatched`/`TimerArmed` — the effect never ran), so the token comes straight from
        /// the requesting `Effect`. `None` = a token-free reducer.
        token: Option<Vec<u8>>,
    },

    /// A FOLD FAILED — the reducer trapped / exhausted fuel / failed to instantiate while folding an
    /// event (error-resilience / supervision direction). §17 totality means a bad reducer can NEVER
    /// panic the kernel loop; instead the failure is CAPTURED as this first-class log event rather than
    /// vanishing into a silent empty fold ("errors into the void"). `reason` is a human/diagnostic string
    /// (the guest trap message / fuel-exhaustion / instantiate error). `caused_event` is the hash of the
    /// event whose fold failed (so a supervisor can see WHAT the reducer choked on). v0 RECORDS it (a
    /// supervisor reading the log sees the failure); it is NOT itself folded (no recursion — a fold that
    /// fails can't be handed back to the same failing reducer). A future supervision slice lets a parent
    /// react (restart/retry/escalate) to a child's FoldFailed.
    FoldFailed { reason: String, caused_event: Hash },

    /// The session closed, carrying a STRUCTURED [`CloseOutcome`] (success-with-payload vs
    /// failure-with-reason). §6 supervision slice-1 (operator directive): a supervisor must be able to tell a
    /// clean completion from a failure to decide restart/retry/escalate — an opaque payload couldn't express
    /// that first-class. When the session had a parent, the kernel delivers this outcome to it as a
    /// `child-completed` signal (slices 2-3, pending the operator's design review).
    Closed { outcome: CloseOutcome },

    /// **DURABLE terminal marker (§lifecycle I1).** A session was TERMINATED by another session (the
    /// `lifecycle/terminate` effect), as distinct from [`Closed`](Self::Closed) (a session ending
    /// ITSELF). Once this is the log TAIL the kernel refuses every further fold ([`KernelError::
    /// FoldRefused`](crate::kernel::KernelError::FoldRefused)) — a first-class guard, not a host
    /// convention, so a terminated session can't be re-driven even by a buggy host. The log + KV are
    /// RETAINED (queryable, frozen); the terminality is durable + replay-stable (a recovered session
    /// whose tail is `Terminated` stays terminated). Terminal: there is no un-terminate (recovery from
    /// a bad state is a fresh spawn, §7).
    ///
    /// `by` is the terminating controller's session identity (its genesis hash = its SessionId); `reason`
    /// is a human/diagnostic string. The supervision-tree authority that gates WHO may terminate is a
    /// host/Cedar concern (I6, `ResourcePredicate::DescendantOf`); this event just records the durable
    /// fact + who did it.
    Terminated { by: Hash, reason: String },

    /// **DURABLE parent→child edge (§lifecycle I2 / §6 supervision).** Recorded in the PARENT's log when it
    /// SPAWNS a child (the `lifecycle/spawn` effect), naming the child by its genesis hash (= the child's
    /// SessionId). These events form the supervision TREE: a controller's transitive `Spawned` descendants
    /// are exactly what its lifecycle authority extends over (I6's `ResourcePredicate::DescendantOf` walks
    /// them) and what a terminate/failure cascade follows (§8). The OTHER half of the relation is the
    /// child's own genesis `parent` field (I2a) — the child-id self-certifies its parent, this edge lets
    /// the parent enumerate its children. Purely a recorded fact (like `FoldFailed`): it is NOT folded
    /// through the reducer (see `observable`) — a supervisor reads it from the log.
    Spawned { child_hash: Hash },

    /// **DURABLE child→parent terminal signal (§6 supervision, V2 per-child).** Recorded in the PARENT's log
    /// when one of its children reaches a terminal outcome — delivered by the host's reap of a self-CLOSED
    /// child AND the `lifecycle/terminate` path (child-exited); ONE variant covers both, the [`CloseOutcome`]
    /// discriminates (self-close = Success|Failure, terminate = Failure(reason)). The symmetric bookend of
    /// [`Spawned`](Self::Spawned) (born → done) on the parent's log. UNLIKE `Spawned` (a recorded fact), this
    /// IS folded through the parent's SUPERVISOR reducer (see `observable`) so it can react PER CHILD —
    /// restart the failed one, count success vs failure, route by `child`. Carrying `child` + `outcome` as
    /// FIRST-CLASS typed fields (surfaced by `build_event_document`) is what lets a `.cdz` guest supervisor
    /// read them directly, rather than value-decoding an opaque `encode_child_completed` payload (which a
    /// guest can't do). `child` is the completed child's genesis hash (= its SessionId).
    ChildCompleted { child: Hash, outcome: CloseOutcome },

    /// A durable CHECKPOINT frame (GAP-4 log-prune-to-checkpoint): a first-class log event carrying the
    /// complete log-derived resident state at its seq, so recovery can resume from `[Genesis, Checkpoint@N,
    /// tail(> N)]` WITHOUT the pruned prefix (genesis stays at `events[0]`; this frame captures everything
    /// `Session::replay` would otherwise re-fold from the dropped frames — see [`CheckpointDescriptor`]).
    /// NOT produced in the normal apply loop; it is APPENDED by the checkpoint/prune path and CONSUMED by
    /// recover-from-checkpoint (later increments). A tuple variant wrapping the descriptor keeps it a cohesive
    /// unit (the same value `Session::build_checkpoint_descriptor` returns + recovery seeds from).
    Checkpoint(CheckpointDescriptor),
}

/// How a session CLOSED — the structured terminal outcome a supervisor acts on (§6 supervision, operator
/// directive: "child-completed carries a structured outcome, success vs failure-with-reason"). Distinguishing
/// success from failure is the minimum a one-for-one supervisor needs to choose restart/retry/escalate; the
/// prior opaque `Payload` couldn't express it. A sum (no sentinel — mirrors [`EffectOutcome`]).
///
/// **Wire compat (both codecs):** `Success` encodes BYTE-IDENTICALLY to the legacy `Closed { outcome: Payload }`
/// (no wrapper tag — a bare payload), so an old `Closed` stream decodes as `Success` unchanged and its event
/// hash / cause edges are preserved; `Failure` takes a fresh discriminant a legacy Payload never produced
/// (binary tag `2`; textual head `failure`). A future arm (e.g. `Cancelled`) likewise takes a fresh unused
/// tag/head — additive — but this is NOT a blanket "tolerant decoder ignores unknowns": both codecs are
/// tag-discriminated and REJECT an unknown tag/head as corruption (the frozen-codec contract).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CloseOutcome {
    /// The session finished its goal cleanly. `payload` is the (opaque) result it produced — what a parent
    /// consumes on a successful child-completed.
    Success(Payload),
    /// The session terminated in failure. `reason` is a human/diagnostic string (why it gave up / what
    /// broke) — what a supervisor logs and reacts to (restart/retry/escalate).
    Failure(String),
}

/// A durable-frame SNAPSHOT of one open (dispatched-but-unsettled) obligation, carried in a checkpoint
/// (GAP-4 log-prune-to-checkpoint, [`CheckpointDescriptor`]). This is the event.rs-native mirror of the
/// kernel's resident `OpenObligation` (kernel.rs): `event.rs` is BELOW `kernel.rs` in the module layering
/// (kernel.rs imports `EventBody`; event.rs must not name kernel types), so the durable frame carries THIS
/// type and the kernel converts `OpenObligation` <-> `CheckpointObligation` at checkpoint-build / recover.
/// Carries the open-table MAP KEY (`id`, the effect id) alongside the frame fields, so recovery rebuilds the
/// resident `BTreeMap<u64, OpenObligation>` exactly. Fields mirror `OpenObligation` (§16c-S1 dispatch record).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CheckpointObligation {
    /// The open-obligation table key: the effect id this obligation is keyed under.
    pub id: u64,
    /// The resolved target the dispatch recorded (opaque bytes; empty for a timer).
    pub target: std::sync::Arc<[u8]>,
    /// The dispatched effect's schema-hash identity; `None` for a register-by-string extension family.
    pub schema_hash: Option<Hash>,
    /// The reducer continuation token the frame carried; `None` for a token-free effect/timer.
    pub token: Option<Vec<u8>>,
    /// A timer obligation's absolute deadline (wall-clock ms); `None` for a non-timer effect.
    pub deadline_ms: Option<u64>,
    /// The hash of the `Dispatched` frame that opened this obligation; `None` for a timer.
    pub dispatch_hash: Option<Hash>,
    /// Is this a TIMER obligation (armed via `TimerArmed`) rather than a dispatched effect?
    pub is_timer: bool,
}

/// The complete DERIVED resident state a checkpoint@N must carry so recovery can resume from
/// `[Genesis, Checkpoint@N, tail(> N)]` WITHOUT the pruned prefix (GAP-4 increment #2 — "what a checkpoint
/// must carry"). Genesis stays UNPRUNED at `events[0]`, so session identity / reducer / provenance are read
/// from `log[0]` and the Genesis-at-`[0]` invariant holds; this descriptor therefore carries ONLY the
/// log-DERIVED state (everything [`crate::kernel::Session::replay`] reconstructs by folding the prefix): the
/// KV root, the id counter, the clock high-water, the settled watermark + sparse exceptions, the
/// open-obligation table (with ids), the spawned-children edges, the capability-seed bit, and the close
/// outcome. Assembled by [`crate::kernel::Session::build_checkpoint_descriptor`]; the durable checkpoint frame
/// wraps it (that frame + its value-form codec is the co-landed increment #1).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CheckpointDescriptor {
    /// The KV Merkle root at the checkpoint seq (seeds the recovered session's KV).
    pub kv_root: Hash,
    /// The next-effect-id counter at the checkpoint (monotonic; recovery resumes id assignment here).
    pub next_effect_id: u64,
    /// The `Now`-clock monotonicity high-water at the checkpoint.
    pub last_now: u64,
    /// The settled-set watermark: every effect id STRICTLY BELOW this is settled.
    pub settled_watermark: u64,
    /// The out-of-order settled ids at/above the watermark (canonically ordered).
    pub settled_exceptions: Vec<u64>,
    /// The resident open-obligation table (dispatched-but-unsettled effects + armed timers), each carrying
    /// its map-key id, so recovery rebuilds the table without the pruned `Dispatched`/`TimerArmed` frames.
    pub open: Vec<CheckpointObligation>,
    /// The spawned-children genesis-hash edges (§6/lifecycle I2/I3) at the checkpoint.
    pub spawned: Vec<Hash>,
    /// Whether the session has already been seeded its capability manifest (host-capability-discovery I3).
    pub seeded_capabilities: bool,
    /// The self-close outcome if the session has closed (terminal), else `None`.
    pub close_outcome: Option<CloseOutcome>,
}

/// Whether a failed effect is worth RETRYING — a FIRST-CLASS typed field on [`EffectOutcome::Err`] (operator
/// Q2), so a reducer's fold matches STRUCTURALLY on the retryability rather than parsing a `RETRYABLE:`/
/// `PERMANENT:` prefix out of the message string (the old host convention this replaces). RETRY POLICY lives
/// in the reducer (re-emit + timer backoff = evolvable-on-log), not the host — the kernel/host only CLASSIFY
/// (e.g. a Bedrock throttle → `Retryable`, a malformed request → `Permanent`); the standing-order split.
/// An enum (not a bool) for additive room (a future `Unknown`/`RetryAfter` arm appends without a wire break).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Retryability {
    /// The failure is permanent — retrying can't help (malformed input, an auth denial, a logic error). The
    /// SAFE DEFAULT: an un-annotated / legacy error is `Permanent` (fail-closed — never auto-retry blindly).
    #[default]
    Permanent,
    /// The failure is transient — a retry (the reducer's policy: backoff + re-emit) may succeed (a throttle,
    /// a timeout, a 5xx). The host CLASSIFIES the error as this; the reducer DECIDES whether/how to retry.
    Retryable,
}

/// The outcome of an executed effect: success payload, a failure, or a timeout (the §9d anti-stuck
/// path — a hung effect becomes a normal event, not a wedge).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum EffectOutcome {
    Ok(Option<Payload>),
    /// A failure: a human/diagnostic `message` + a typed [`Retryability`] (operator Q2 — the reducer folds
    /// on the retryability, not a string token). Construct a permanent error ergonomically via
    /// [`EffectOutcome::err`]; a retryable one via [`EffectOutcome::err_retryable`].
    Err {
        message: String,
        retryability: Retryability,
    },
    /// The effect's deadline elapsed with no result. Per the v0 decision (§16c-S4), a timeout CANCELS
    /// the dispatch — the kernel guarantees no late `Ok`/`Err` for this id will ever be folded.
    TimedOut,
    /// The executor accepted the effect but will NOT answer synchronously — a later
    /// [`crate::kernel::Session::settle_effect_result`] (by [`EffectId`]) folds the real outcome back
    /// (userspace-effects I2). An executor returns this from `perform` when it FORWARDED the effect for
    /// asynchronous fulfillment — e.g. a `UserspaceEffectExecutor` that delegated to a registered handler
    /// session, or the host reflecting a `control/signature` off-band. The kernel leaves the effect's
    /// `Dispatched` frame OPEN (does NOT `record_result`) so the continuation resumes later on the settle.
    ///
    /// This is a TRANSIENT executor→kernel signal, NEVER a folded result: it is intercepted on the routed
    /// path before `record_result`, so it never becomes an `EffectResult` on the log and the codec never
    /// encodes it (the eventual `settle_effect_result` folds a real `Ok`/`Err`). The defer decision is
    /// RUNTIME state only the executor's `perform` can see (is a handler registered for this family?), which
    /// is why it is an executor RETURN value, not a static dispatch-site predicate.
    Deferred,
}

impl EffectOutcome {
    /// A PERMANENT failure (the common case — a malformed effect, a no-store, an internal error): retrying
    /// can't help. The ergonomic constructor the ~all kernel-internal error sites use (they were `Err(msg)`
    /// before Q2 added the typed field; `err(msg)` preserves their intent = `Permanent`).
    pub fn err(message: impl Into<String>) -> Self {
        EffectOutcome::Err {
            message: message.into(),
            retryability: Retryability::Permanent,
        }
    }

    /// A RETRYABLE failure (the host sets this when it classifies a transient error — a throttle/timeout/5xx);
    /// the reducer's fold decides whether/how to retry (backoff + re-emit).
    pub fn err_retryable(message: impl Into<String>) -> Self {
        EffectOutcome::Err {
            message: message.into(),
            retryability: Retryability::Retryable,
        }
    }
}

/// An event plus its envelope. The envelope fields are the day-one commitments from the review/§10:
/// `cause` for the causal DAG, room for `sig`/`producer` (added when multi-operator lands).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Event {
    pub seq: SeqNo,
    /// The causal parent — the event (possibly in another session) that led to this one (§5). `None`
    /// for genesis and for un-caused external ingress.
    pub cause: Option<Hash>,
    pub body: EventBody,
}

impl Event {
    /// Content-address this event's canonical bytes. This is what `cause` edges and the log's
    /// tamper-evidence point at. v0 uses a simple deterministic encoding; §16c-S3 requires this
    /// encoding be frozen/canonical, which we honor by keeping it explicit and total here.
    pub fn hash(&self) -> Hash {
        Hash::of(&self.encode())
    }

    /// Canonical byte encoding (frozen — §16c-S3). Deterministic: same event → same bytes → same hash.
    /// Kept intentionally simple and self-contained (no serde) so the canonical form is auditable in
    /// one place. This is the on-disk log format (durable log persistence, next slice): what `encode`
    /// writes, [`Event::decode`] reads back exactly.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.seq.to_le_bytes());
        match &self.cause {
            Some(h) => {
                out.push(1);
                out.extend_from_slice(h.as_bytes());
            }
            None => out.push(0),
        }
        encode_body(&self.body, &mut out);
        out
    }

    /// Decode an event from its canonical bytes (the inverse of [`Event::encode`]). Total: any
    /// malformed/truncated input yields `Err`, never a panic (the durable log must survive a torn
    /// write at the tail — the reader stops cleanly at the last well-formed event). Returns the event
    /// AND the number of bytes consumed, so a log reader can decode a concatenated stream.
    pub fn decode(bytes: &[u8]) -> Result<(Event, usize), DecodeError> {
        let mut c = Cursor::new(bytes);
        let seq = c.u64()?;
        let cause = match c.u8()? {
            0 => None,
            1 => Some(Hash::from_bytes(c.hash()?)),
            t => {
                return Err(DecodeError::BadTag {
                    field: "cause",
                    tag: t,
                })
            }
        };
        let body = decode_body(&mut c)?;
        Ok((Event { seq, cause, body }, c.pos))
    }
}

/// A decode failure. The durable log reader treats any of these at the stream tail as "stop here"
/// (a torn final write), and anywhere else as genuine corruption to surface.
#[derive(Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// Ran out of bytes mid-field (truncated / torn write).
    Truncated,
    /// A variant tag byte that isn't a known variant (corruption or a newer format — v0 rejects).
    BadTag { field: &'static str, tag: u8 },
    /// A length field claims more bytes than remain.
    BadLength,
    /// A string field wasn't valid UTF-8.
    BadUtf8,
}

/// A minimal forward-only byte cursor for decoding. Every read is bounds-checked → total decode.
struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Cursor { bytes, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.pos.checked_add(n).ok_or(DecodeError::BadLength)?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or(DecodeError::Truncated)?;
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }

    /// The next byte WITHOUT consuming it (for a tag-discriminated sum whose variant then re-reads the tag —
    /// e.g. `CloseOutcome`, whose `Success` delegates to `decode_payload` which re-reads the 0/1 payload tag).
    fn peek_u8(&self) -> Result<u8, DecodeError> {
        self.bytes
            .get(self.pos)
            .copied()
            .ok_or(DecodeError::Truncated)
    }

    fn u32(&mut self) -> Result<u32, DecodeError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64, DecodeError> {
        let b = self.take(8)?;
        let mut arr = [0u8; 8];
        arr.copy_from_slice(b);
        Ok(u64::from_le_bytes(arr))
    }

    fn hash(&mut self) -> Result<[u8; 32], DecodeError> {
        let b = self.take(32)?;
        let mut arr = [0u8; 32];
        arr.copy_from_slice(b);
        Ok(arr)
    }

    /// Read a `u64` length prefix and convert to `usize`, FAILING with `BadLength` if it doesn't fit
    /// (PR#990 finding #3). The length comes from the untrusted durable log; a bare `as usize` would
    /// TRUNCATE on a 32-bit target (a huge length wraps small → mis-parse). This rejects the frame
    /// cleanly instead. A too-large length then errs one of two ways at the subsequent `take` (PR#993
    /// #3): `Truncated` if it merely exceeds the remaining bytes, or `BadLength` if `pos + len`
    /// overflows `usize` (take's own checked-add guard) — both clean rejections, never a panic or
    /// mis-parse.
    fn len(&mut self) -> Result<usize, DecodeError> {
        usize::try_from(self.u64()?).map_err(|_| DecodeError::BadLength)
    }

    /// A length-prefixed UTF-8 string (matches `encode_str`).
    fn string(&mut self) -> Result<String, DecodeError> {
        let len = self.len()?;
        let b = self.take(len)?;
        core::str::from_utf8(b)
            .map(|s| s.to_string())
            .map_err(|_| DecodeError::BadUtf8)
    }

    /// Read a length-prefixed RAW byte string — the same u64-len + bytes framing as [`string`](Self::string)
    /// but WITHOUT UTF-8 validation (operator Target=Bytes ruling: an effect target is opaque bytes, not
    /// necessarily UTF-8). Wire-compatible with a `string()`-encoded value (a str was already its UTF-8
    /// bytes), so a pre-ruling log decodes identically.
    fn bytes(&mut self) -> Result<Vec<u8>, DecodeError> {
        let len = self.len()?;
        Ok(self.take(len)?.to_vec())
    }
}

fn decode_body(c: &mut Cursor) -> Result<EventBody, DecodeError> {
    let tag = c.u8()?;
    Ok(match tag {
        0 => EventBody::Genesis {
            reducer: Hash::from_bytes(c.hash()?),
            spawn_nonce: Hash::from_bytes(c.hash()?),
            parent: match c.u8()? {
                0 => None,
                1 => Some(Hash::from_bytes(c.hash()?)),
                t => {
                    return Err(DecodeError::BadTag {
                        field: "genesis.parent",
                        tag: t,
                    })
                }
            },
        },
        1 => {
            let family = c.string()?;
            let version = c.u32()?;
            let payload = decode_payload(c)?;
            EventBody::Inbound {
                content_type: ContentType {
                    family: family.into(),
                    version,
                },
                payload,
            }
        }
        2 => {
            let id = EffectId(c.u64()?);
            // Target is opaque bytes (Target=Bytes ruling) — read raw, no UTF-8 validation. Wire-compatible
            // with the old `string()` encoding (a str was its UTF-8 bytes under the same len-prefix framing).
            let target = c.bytes()?;
            let idempotency_key = Hash::from_bytes(c.hash()?);
            let deadline_ms = decode_opt_u64(c)?;
            let token = decode_opt_bytes(c)?;
            // Schema-hash-only (slice-2): the frame's identity is the schema_hash, read HERE (kind tag +
            // family string dropped — see the encode counterpart). Option: None for a register-by-string
            // extension with no wire hash yet (phase-3); read as opt-bytes → Hash.
            let schema_hash = decode_opt_bytes(c)?
                .map(|b| {
                    <[u8; 32]>::try_from(b.as_slice())
                        .map(Hash::from_bytes)
                        .map_err(|_| DecodeError::BadTag {
                            field: "schema_hash",
                            tag: b.len() as u8,
                        })
                })
                .transpose()?;
            EventBody::Dispatched {
                id,
                target: std::sync::Arc::from(target.as_slice()),
                idempotency_key,
                deadline_ms,
                token,
                schema_hash,
            }
        }
        3 => EventBody::EffectResult {
            id: EffectId(c.u64()?),
            result: decode_outcome(c)?,
            token: decode_opt_bytes(c)?,
        },
        4 => EventBody::TimerArmed {
            id: EffectId(c.u64()?),
            deadline_ms: c.u64()?,
            token: decode_opt_bytes(c)?,
        },
        5 => EventBody::TimerFired {
            id: EffectId(c.u64()?),
            fired_ms: c.u64()?,
            token: decode_opt_bytes(c)?,
        },
        6 => EventBody::AuthzDenied {
            id: EffectId(c.u64()?),
            reason: c.string()?,
            token: decode_opt_bytes(c)?,
        },
        7 => EventBody::Closed {
            outcome: decode_close_outcome(c)?,
        },
        8 => EventBody::FoldFailed {
            reason: c.string()?,
            caused_event: Hash::from_bytes(c.hash()?),
        },
        9 => EventBody::Terminated {
            by: Hash::from_bytes(c.hash()?),
            reason: c.string()?,
        },
        10 => EventBody::Spawned {
            child_hash: Hash::from_bytes(c.hash()?),
        },
        11 => EventBody::ChildCompleted {
            child: Hash::from_bytes(c.hash()?),
            outcome: decode_close_outcome(c)?,
        },
        12 => EventBody::Checkpoint(decode_checkpoint_descriptor(c)?),
        t => {
            return Err(DecodeError::BadTag {
                field: "body",
                tag: t,
            })
        }
    })
}

/// Decode a 32-byte hash written as opt-bytes (`None` presence → `None`; `Some` → exactly 32 bytes). The
/// shared reader for the checkpoint frame's `schema_hash` / `dispatch_hash` fields (mirrors the Dispatched
/// frame's `schema_hash` opt-bytes→Hash read). A non-32 length is corruption (`BadLength`).
fn decode_opt_hash(c: &mut Cursor) -> Result<Option<Hash>, DecodeError> {
    decode_opt_bytes(c)?
        .map(|b| {
            <[u8; 32]>::try_from(b.as_slice())
                .map(Hash::from_bytes)
                .map_err(|_| DecodeError::BadLength)
        })
        .transpose()
}

/// Decode a [`CheckpointDescriptor`] — the dual of [`encode_checkpoint_descriptor`], reading the 9 fields in
/// struct-declaration order. Lists read their `u64` length (via `Cursor::len`, which fails `BadLength` on an
/// oversized untrusted count) then that many elements. `is_timer` / `seeded_capabilities` / the close-outcome
/// presence byte reject a non-0/1 tag as corruption (the frozen-codec contract).
fn decode_checkpoint_descriptor(c: &mut Cursor) -> Result<CheckpointDescriptor, DecodeError> {
    let kv_root = Hash::from_bytes(c.hash()?);
    let next_effect_id = c.u64()?;
    let last_now = c.u64()?;
    let settled_watermark = c.u64()?;
    let n_exc = c.len()?;
    let mut settled_exceptions = Vec::with_capacity(n_exc);
    for _ in 0..n_exc {
        settled_exceptions.push(c.u64()?);
    }
    let n_open = c.len()?;
    let mut open = Vec::with_capacity(n_open);
    for _ in 0..n_open {
        let id = c.u64()?;
        let target: std::sync::Arc<[u8]> = c.bytes()?.into();
        let schema_hash = decode_opt_hash(c)?;
        let token = decode_opt_bytes(c)?;
        let deadline_ms = decode_opt_u64(c)?;
        let dispatch_hash = decode_opt_hash(c)?;
        let is_timer = match c.u8()? {
            0 => false,
            1 => true,
            t => {
                return Err(DecodeError::BadTag {
                    field: "checkpoint.obligation.is_timer",
                    tag: t,
                })
            }
        };
        open.push(CheckpointObligation {
            id,
            target,
            schema_hash,
            token,
            deadline_ms,
            dispatch_hash,
            is_timer,
        });
    }
    let n_spawned = c.len()?;
    let mut spawned = Vec::with_capacity(n_spawned);
    for _ in 0..n_spawned {
        spawned.push(Hash::from_bytes(c.hash()?));
    }
    let seeded_capabilities = match c.u8()? {
        0 => false,
        1 => true,
        t => {
            return Err(DecodeError::BadTag {
                field: "checkpoint.seeded_capabilities",
                tag: t,
            })
        }
    };
    let close_outcome = match c.u8()? {
        0 => None,
        1 => Some(decode_close_outcome(c)?),
        t => {
            return Err(DecodeError::BadTag {
                field: "checkpoint.close_outcome",
                tag: t,
            })
        }
    };
    Ok(CheckpointDescriptor {
        kv_root,
        next_effect_id,
        last_now,
        settled_watermark,
        settled_exceptions,
        open,
        spawned,
        seeded_capabilities,
        close_outcome,
    })
}

fn decode_opt_u64(c: &mut Cursor) -> Result<Option<u64>, DecodeError> {
    Ok(match c.u8()? {
        0 => None,
        1 => Some(c.u64()?),
        t => {
            return Err(DecodeError::BadTag {
                field: "opt_u64",
                tag: t,
            })
        }
    })
}

fn decode_opt_bytes(c: &mut Cursor) -> Result<Option<Vec<u8>>, DecodeError> {
    Ok(match c.u8()? {
        0 => None,
        1 => {
            let len = c.len()?;
            Some(c.take(len)?.to_vec())
        }
        t => {
            return Err(DecodeError::BadTag {
                field: "opt_bytes",
                tag: t,
            })
        }
    })
}

fn decode_payload(c: &mut Cursor) -> Result<Payload, DecodeError> {
    Ok(match c.u8()? {
        0 => {
            let len = c.len()?;
            Payload::Inline(c.take(len)?.to_vec().into())
        }
        1 => Payload::Blob(Hash::from_bytes(c.hash()?)),
        t => {
            return Err(DecodeError::BadTag {
                field: "payload",
                tag: t,
            })
        }
    })
}

fn decode_outcome(c: &mut Cursor) -> Result<EffectOutcome, DecodeError> {
    Ok(match c.u8()? {
        0 => EffectOutcome::Ok(match c.u8()? {
            0 => None,
            1 => Some(decode_payload(c)?),
            t => {
                return Err(DecodeError::BadTag {
                    field: "outcome_ok",
                    tag: t,
                })
            }
        }),
        1 => {
            let message = c.string()?;
            let retryability = match c.u8()? {
                0 => Retryability::Permanent,
                1 => Retryability::Retryable,
                t => {
                    return Err(DecodeError::BadTag {
                        field: "retryability",
                        tag: t,
                    })
                }
            };
            EffectOutcome::Err {
                message,
                retryability,
            }
        }
        2 => EffectOutcome::TimedOut,
        t => {
            return Err(DecodeError::BadTag {
                field: "outcome",
                tag: t,
            })
        }
    })
}

/// Encode an event body canonically. A leading tag byte per variant keeps the form unambiguous and
/// stable. (When the wire format migrates to canonical s-expr per the design, this is the one place
/// that changes — and it must stay frozen thereafter.)
fn encode_body(body: &EventBody, out: &mut Vec<u8>) {
    match body {
        EventBody::Genesis {
            reducer,
            spawn_nonce,
            parent,
        } => {
            out.push(0);
            out.extend_from_slice(reducer.as_bytes());
            out.extend_from_slice(spawn_nonce.as_bytes());
            // parent: presence byte + 32 hash bytes when Some (mirrors the Event.cause Option<Hash> wire).
            match parent {
                Some(p) => {
                    out.push(1);
                    out.extend_from_slice(p.as_bytes());
                }
                None => out.push(0),
            }
        }
        EventBody::Inbound {
            content_type,
            payload,
        } => {
            out.push(1);
            encode_str(&content_type.family, out);
            out.extend_from_slice(&content_type.version.to_le_bytes());
            encode_payload(payload, out);
        }
        EventBody::Dispatched {
            id,
            target,
            idempotency_key,
            deadline_ms,
            token,
            // Schema-hash-only (slice-2): the frame's identity is `schema_hash`, written HERE (the legacy
            // `kind` tag + `family` string were dropped). slice-D=A (abandon old logs) so this codec change
            // is not a back-compat break. `Event::hash` now covers the schema-hash instead of kind/family.
            schema_hash,
        } => {
            out.push(2);
            out.extend_from_slice(&id.0.to_le_bytes());
            encode_bytes(target, out);
            out.extend_from_slice(idempotency_key.as_bytes());
            encode_opt_u64(*deadline_ms, out);
            encode_opt_bytes(token.as_deref(), out);
            // schema_hash is Option (None for a register-by-string extension with no wire hash yet — phase-3);
            // encode as opt-bytes (the raw 32 bytes when Some).
            encode_opt_bytes(schema_hash.as_ref().map(|h| h.as_bytes().as_slice()), out);
        }
        EventBody::EffectResult { id, result, token } => {
            out.push(3);
            out.extend_from_slice(&id.0.to_le_bytes());
            encode_outcome(result, out);
            encode_opt_bytes(token.as_deref(), out);
        }
        EventBody::TimerArmed {
            id,
            deadline_ms,
            token,
        } => {
            out.push(4);
            out.extend_from_slice(&id.0.to_le_bytes());
            out.extend_from_slice(&deadline_ms.to_le_bytes());
            encode_opt_bytes(token.as_deref(), out);
        }
        EventBody::TimerFired {
            id,
            fired_ms,
            token,
        } => {
            out.push(5);
            out.extend_from_slice(&id.0.to_le_bytes());
            out.extend_from_slice(&fired_ms.to_le_bytes());
            encode_opt_bytes(token.as_deref(), out);
        }
        EventBody::AuthzDenied { id, reason, token } => {
            out.push(6);
            out.extend_from_slice(&id.0.to_le_bytes());
            encode_str(reason, out);
            encode_opt_bytes(token.as_deref(), out);
        }
        EventBody::Closed { outcome } => {
            out.push(7);
            encode_close_outcome(outcome, out);
        }
        EventBody::FoldFailed {
            reason,
            caused_event,
        } => {
            out.push(8);
            encode_str(reason, out);
            out.extend_from_slice(caused_event.as_bytes());
        }
        EventBody::Terminated { by, reason } => {
            out.push(9);
            out.extend_from_slice(by.as_bytes());
            encode_str(reason, out);
        }
        EventBody::Spawned { child_hash } => {
            out.push(10);
            out.extend_from_slice(child_hash.as_bytes());
        }
        EventBody::ChildCompleted { child, outcome } => {
            out.push(11);
            out.extend_from_slice(child.as_bytes());
            encode_close_outcome(outcome, out);
        }
        EventBody::Checkpoint(d) => {
            out.push(12);
            encode_checkpoint_descriptor(d, out);
        }
    }
}

/// Encode a [`CheckpointDescriptor`] (GAP-4 checkpoint frame) — the 9 log-derived fields in
/// struct-declaration order; lists are a `u64` length prefix then their elements. Nested under the
/// `Checkpoint` frame tag. (The value-form/event_ast codec mirrors this shape + field order; event_ast is
/// v-cml's single-writer lane.)
fn encode_checkpoint_descriptor(d: &CheckpointDescriptor, out: &mut Vec<u8>) {
    out.extend_from_slice(d.kv_root.as_bytes());
    out.extend_from_slice(&d.next_effect_id.to_le_bytes());
    out.extend_from_slice(&d.last_now.to_le_bytes());
    out.extend_from_slice(&d.settled_watermark.to_le_bytes());
    // settled_exceptions: len prefix + each id (ascending, as produced by SettledSet::exceptions).
    out.extend_from_slice(&(d.settled_exceptions.len() as u64).to_le_bytes());
    for id in &d.settled_exceptions {
        out.extend_from_slice(&id.to_le_bytes());
    }
    // open: len prefix + each obligation's 7 fields in declaration order.
    out.extend_from_slice(&(d.open.len() as u64).to_le_bytes());
    for ob in &d.open {
        out.extend_from_slice(&ob.id.to_le_bytes());
        encode_bytes(&ob.target, out);
        encode_opt_bytes(
            ob.schema_hash.as_ref().map(|h| h.as_bytes().as_slice()),
            out,
        );
        encode_opt_bytes(ob.token.as_deref(), out);
        encode_opt_u64(ob.deadline_ms, out);
        encode_opt_bytes(
            ob.dispatch_hash.as_ref().map(|h| h.as_bytes().as_slice()),
            out,
        );
        out.push(ob.is_timer as u8);
    }
    // spawned: len prefix + each 32-byte genesis hash.
    out.extend_from_slice(&(d.spawned.len() as u64).to_le_bytes());
    for h in &d.spawned {
        out.extend_from_slice(h.as_bytes());
    }
    out.push(d.seeded_capabilities as u8);
    // close_outcome: presence byte + the CloseOutcome codec when Some.
    match &d.close_outcome {
        Some(o) => {
            out.push(1);
            encode_close_outcome(o, out);
        }
        None => out.push(0),
    }
}

fn encode_str(s: &str, out: &mut Vec<u8>) {
    out.extend_from_slice(&(s.len() as u64).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

/// Encode a length-prefixed RAW byte string — the same u64-len + bytes framing as [`encode_str`], for an
/// opaque byte field (an effect target — Target=Bytes ruling). Byte-identical to `encode_str` for a value
/// that happens to be UTF-8, so a target's on-wire shape is unchanged from when it was an `Arc<str>`.
fn encode_bytes(b: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(&(b.len() as u64).to_le_bytes());
    out.extend_from_slice(b);
}

fn encode_opt_u64(v: Option<u64>, out: &mut Vec<u8>) {
    match v {
        Some(n) => {
            out.push(1);
            out.extend_from_slice(&n.to_le_bytes());
        }
        None => out.push(0),
    }
}

/// Optional opaque byte string (a reducer's continuation token, §19e): `0` = None, `1` + u64-len +
/// bytes = Some. Same present-tag + length-prefix shape as the other opt/str encoders.
fn encode_opt_bytes(v: Option<&[u8]>, out: &mut Vec<u8>) {
    match v {
        Some(b) => {
            out.push(1);
            out.extend_from_slice(&(b.len() as u64).to_le_bytes());
            out.extend_from_slice(b);
        }
        None => out.push(0),
    }
}

fn encode_payload(p: &Payload, out: &mut Vec<u8>) {
    match p {
        Payload::Inline(bytes) => {
            out.push(0);
            out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
            out.extend_from_slice(bytes);
        }
        Payload::Blob(h) => {
            out.push(1);
            out.extend_from_slice(h.as_bytes());
        }
    }
}

fn encode_outcome(o: &EffectOutcome, out: &mut Vec<u8>) {
    match o {
        EffectOutcome::Ok(p) => {
            out.push(0);
            match p {
                Some(p) => {
                    out.push(1);
                    encode_payload(p, out);
                }
                None => out.push(0),
            }
        }
        EffectOutcome::Err {
            message,
            retryability,
        } => {
            out.push(1);
            encode_str(message, out);
            // Retryability rides as a fixed byte AFTER the message (this bespoke binary codec is NOT the
            // durable log — that's event_ast; here always-write is safe + keeps decode positional).
            out.push(match retryability {
                Retryability::Permanent => 0,
                Retryability::Retryable => 1,
            });
        }
        EffectOutcome::TimedOut => out.push(2),
        // Deferred is a transient executor→kernel signal, never a settled result: the routed path intercepts
        // it before `record_result`, so it never becomes an `EffectResult` and thus never reaches this codec.
        EffectOutcome::Deferred => {
            unreachable!(
                "EffectOutcome::Deferred is never recorded/encoded — it is intercepted pre-record"
            )
        }
    }
}

// BACKWARD-COMPATIBLE with the legacy `Closed { outcome: Payload }` wire (frozen-codec discipline;
// fix-forward on #1938 which shipped a wire-breaking wrapper tag): legacy encoded a bare `Payload` after the
// `7` tag (its own inner tag 0=Inline / 1=Blob). So `Success` encodes BYTE-IDENTICALLY to a legacy Payload
// (NO extra wrapper tag) — an old `Closed` stream decodes as `Success` unchanged, its event hash preserved —
// and `Failure` takes a FRESH tag `2` that a legacy Payload's leading byte never was.
fn encode_close_outcome(o: &CloseOutcome, out: &mut Vec<u8>) {
    match o {
        // No wrapper tag: emit exactly the legacy Payload bytes (leading 0 or 1). Old readers/hashes match.
        CloseOutcome::Success(p) => encode_payload(p, out),
        CloseOutcome::Failure(reason) => {
            out.push(2);
            encode_str(reason, out);
        }
    }
}

fn decode_close_outcome(c: &mut Cursor) -> Result<CloseOutcome, DecodeError> {
    // Peek the leading byte WITHOUT consuming it: 0/1 = a legacy Payload → Success (so old streams parse);
    // 2 = the new Failure tag. `decode_payload` re-reads the 0/1 tag itself, so Success delegates to it.
    Ok(match c.peek_u8()? {
        0 | 1 => CloseOutcome::Success(decode_payload(c)?),
        2 => {
            c.u8()?; // consume the Failure tag
            CloseOutcome::Failure(c.string()?)
        }
        t => {
            return Err(DecodeError::BadTag {
                field: "close_outcome",
                tag: t,
            })
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn genesis() -> Event {
        Event {
            seq: 0,
            cause: None,
            body: EventBody::Genesis {
                reducer: Hash::of(b"reducer-v1"),
                spawn_nonce: Hash::of(b"nonce-v1"),
                parent: None,
            },
        }
    }

    #[test]
    fn event_hash_is_deterministic() {
        assert_eq!(genesis().hash(), genesis().hash());
    }

    #[test]
    fn report_content_type_is_the_well_known_report_family() {
        let ct = ContentType::report();
        assert_eq!(ct.family, "report");
        assert_eq!(ct.family, ContentType::REPORT_FAMILY);
        assert_eq!(ct.version, 1);
        assert!(ct.is_report());
    }

    #[test]
    fn is_report_matches_family_only_and_rejects_other_families() {
        // Version-tolerant (§9b): a different report version is still "a report" — the reducer
        // range-checks version itself if it cares.
        let other_version = ContentType {
            family: "report".into(),
            version: 7,
        };
        assert!(other_version.is_report());
        // A different family is NOT a report.
        let message = ContentType {
            family: "message".into(),
            version: 1,
        };
        assert!(!message.is_report());
    }

    #[test]
    fn matches_family_and_version_in_are_the_tolerant_reader_split() {
        // The §9b tolerant-reader split: family match is version-INDEPENDENT, and the version range is a
        // SEPARATE inclusive check — so "known family, unknown version" is representable (match family,
        // fail version) rather than collapsing into one decode-or-die test.
        let ct = ContentType {
            family: "model-request".into(),
            version: 2,
        };
        // Family match ignores version...
        assert!(ct.matches_family("model-request"));
        assert!(!ct.matches_family("model-response"));
        // ...and the version range is inclusive on both ends.
        assert!(ct.version_in(1, 3));
        assert!(ct.version_in(2, 2));
        assert!(!ct.version_in(3, 5)); // below the range
        assert!(!ct.version_in(0, 1)); // above the range
                                       // The "known family, unknown version" case a v1 reader must DEFER, not misdecode: family matches
                                       // but the version is outside what it handles — the two checks disagree, which is the whole point.
        assert!(ct.matches_family("model-request") && !ct.version_in(1, 1));
        // Totality: an empty/backwards range is simply never satisfied (no panic).
        assert!(!ct.version_in(5, 1));
        // is_report is exactly matches_family(REPORT_FAMILY) — one source of truth.
        let r = ContentType::report();
        assert_eq!(r.is_report(), r.matches_family(ContentType::REPORT_FAMILY));
    }

    // A FULLY-FIXED event (every field a literal, no Hash::of indirection) whose encoded bytes are
    // pinned by the frozen-encoding golden test below.
    fn golden_event() -> Event {
        Event {
            seq: 42,
            cause: Some(Hash::from_bytes([0xABu8; 32])),
            body: EventBody::Dispatched {
                id: EffectId(7),
                target: "https://ok.host/p".as_bytes().into(),
                idempotency_key: Hash::from_bytes([0xCDu8; 32]),
                deadline_ms: Some(1000),
                token: Some(b"step-1".to_vec()),
                schema_hash: Some(crate::ast_marshal::builtin_effect_schema_hash(
                    &crate::effect::EffectKind::Http,
                )),
            },
        }
    }

    #[test]
    fn frozen_encoding_is_byte_stable_golden() {
        // §16c-S3: the encoding is FROZEN — "same event → same bytes → same hash." The round-trip test
        // proves decode(encode(x)) == x, but NOT that encode(x) yields the SAME bytes over time: a
        // refactor could change the byte layout while keeping round-trip intact, silently invalidating
        // every persisted log's content-address hashes + `cause` edges (a session that recovers under a
        // new layout would compute different hashes for the same history). This golden pin is the
        // tripwire: if it fails, the on-disk format changed — that MUST be a conscious, versioned
        // migration (e.g. the planned swap to the shared `cadenza-ast` codec), never an accident.
        //
        // Byte layout of `golden_event()` (little-endian; the frozen §16c-S3 format):
        //   seq=42                → 42,0,0,0,0,0,0,0        (u64 LE)
        //   cause=Some(0xAB..)    → 1, then 32×0xAB(171)    (tag + Hash)
        //   body tag=Dispatched   → 2
        //   id=7                  → 7,0,0,0,0,0,0,0         (u64 LE)
        //   target len=17         → 17,0,0,0,0,0,0,0        (u64 LE len)
        //   target="https://ok.host/p" → its 17 UTF-8 bytes
        //   idempotency_key=0xCD.. → 32×0xCD(205)           (Hash)
        //   deadline_ms=Some(1000) → 1, then 1000 as u64 LE (232,3,0,0,0,0,0,0)
        //   token=Some("step-1")  → 1, then 6 as u64 LE, then "step-1"
        //   schema_hash=Some(Http builtin) → opt-bytes: 1, len 32, then its 32 bytes (…ends 26,15)
        //
        // SCHEMA-HASH-ONLY (slice-2) CONSCIOUS format change: the Dispatched frame DROPPED the `kind` tag +
        // the `family` str (they were the pre-slice-2 identity) and now carries the `schema_hash` (raw 32
        // bytes) as the SOLE identity, written after `token`. slice-D=A (abandon old logs, no migration
        // layer) makes this pre-release in-place format change deliberate, not an accidental break — this
        // golden pin re-freezes the NEW byte-stable layout. (Prior deliberate bumps: §19e trailing opt-bytes
        // `token` on tags 2/3/4/5/6; the now-removed host-capability-discovery `family` str after `kind`.)
        let expected: &[u8] = &[
            42, 0, 0, 0, 0, 0, 0, 0, // seq
            1, // cause: Some
            171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171,
            171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171,
            171, // cause hash
            2,   // body tag = Dispatched
            7, 0, 0, 0, 0, 0, 0, 0, // id
            17, 0, 0, 0, 0, 0, 0, 0, // target len
            104, 116, 116, 112, 115, 58, 47, 47, 111, 107, 46, 104, 111, 115, 116, 47,
            112, // "https://ok.host/p"
            205, 205, 205, 205, 205, 205, 205, 205, 205, 205, 205, 205, 205, 205, 205, 205, 205,
            205, 205, 205, 205, 205, 205, 205, 205, 205, 205, 205, 205, 205, 205,
            205, // idempotency_key hash
            1, 232, 3, 0, 0, 0, 0, 0, 0, // deadline_ms: Some(1000)
            1, 6, 0, 0, 0, 0, 0, 0, 0, // token: Some, len 6
            115, 116, 101, 112, 45, 49, // "step-1"
            // schema_hash: Option → opt-bytes: Some tag(1) + len 32 + the Http built-in schema-hash 32 bytes
            1, 32, 0, 0, 0, 0, 0, 0, 0, // schema_hash: Some, len 32
            50, 211, 199, 222, 97, 236, 120, 70, 26, 123, 238, 204, 128, 12, 197, 71, 153, 253,
            253, 37, 82, 45, 89, 30, 10, 191, 156, 49, 213, 191, 26, 15,
        ];
        assert_eq!(
            golden_event().encode(),
            expected,
            "FROZEN ENCODING CHANGED — the on-disk log format is not byte-stable. If this is an \
             intentional format change it MUST be a versioned migration (§16c-S3), not an accident."
        );
        // And the content-address hash the log/cause-edges depend on is pinned too.
        assert_eq!(
            golden_event().hash().to_hex(),
            "8b1bcc1f06d207b375b604c6a60129612097783ce39366c2af78c9580fd6e522",
            "FROZEN event hash changed — persisted `cause` edges + content addresses would break. \
             (Re-frozen for the schema-hash-only Dispatched format: kind/family dropped, schema_hash added.)"
        );
    }

    #[test]
    fn seq_change_changes_hash() {
        let mut e = genesis();
        e.seq = 1;
        assert_ne!(e.hash(), genesis().hash());
    }

    #[test]
    fn distinct_bodies_hash_distinctly() {
        let a = Event {
            seq: 5,
            cause: None,
            body: EventBody::TimerFired {
                id: EffectId(1),
                fired_ms: 100,
                token: None,
            },
        };
        let b = Event {
            seq: 5,
            cause: None,
            body: EventBody::TimerFired {
                id: EffectId(1),
                fired_ms: 101,
                token: None,
            },
        };
        assert_ne!(a.hash(), b.hash());
    }

    #[test]
    fn cause_edge_is_part_of_identity() {
        let mut e = genesis();
        e.body = EventBody::Closed {
            outcome: CloseOutcome::Success(Payload::Inline(vec![].into())),
        };
        let without = e.hash();
        e.cause = Some(Hash::of(b"parent"));
        assert_ne!(e.hash(), without);
    }

    /// Every event body variant, for exhaustive round-trip coverage. If a new variant is added,
    /// `encode_body`/`decode_body` must both handle it — this list is the reminder.
    fn all_variants() -> Vec<Event> {
        let h = Hash::of(b"x");
        let ct = ContentType {
            family: "message".into(),
            version: 3,
        };
        vec![
            Event {
                seq: 0,
                cause: None,
                body: EventBody::Genesis {
                    reducer: h,
                    spawn_nonce: Hash::of(b"spawn-nonce"),
                    parent: Some(Hash::of(b"parent-genesis")),
                },
            },
            Event {
                seq: 1,
                cause: Some(h),
                body: EventBody::Inbound {
                    content_type: ct.clone(),
                    payload: Payload::Inline(b"hello".to_vec().into()),
                },
            },
            Event {
                seq: 2,
                cause: None,
                body: EventBody::Inbound {
                    content_type: ct,
                    payload: Payload::Blob(h),
                },
            },
            Event {
                seq: 3,
                cause: Some(h),
                body: EventBody::Dispatched {
                    id: EffectId(7),
                    target: "https://ok.host/p".as_bytes().into(),
                    idempotency_key: h,
                    deadline_ms: Some(12345),
                    token: Some(b"resume-tok".to_vec()),
                    schema_hash: Some(crate::ast_marshal::builtin_effect_schema_hash(
                        &crate::effect::EffectKind::Http,
                    )),
                },
            },
            Event {
                seq: 4,
                cause: None,
                body: EventBody::Dispatched {
                    id: EffectId(8),
                    target: "cargo test".as_bytes().into(),
                    idempotency_key: h,
                    deadline_ms: None,
                    token: None,
                    schema_hash: Some(crate::ast_marshal::builtin_effect_schema_hash(
                        &crate::effect::EffectKind::Shell,
                    )),
                },
            },
            Event {
                seq: 5,
                cause: None,
                body: EventBody::EffectResult {
                    id: EffectId(7),
                    result: EffectOutcome::Ok(Some(Payload::Inline(b"body".to_vec().into()))),
                    token: Some(b"resume-tok".to_vec()),
                },
            },
            Event {
                seq: 6,
                cause: None,
                body: EventBody::EffectResult {
                    id: EffectId(8),
                    result: EffectOutcome::Ok(None),
                    token: None,
                },
            },
            Event {
                seq: 7,
                cause: None,
                body: EventBody::EffectResult {
                    id: EffectId(9),
                    result: EffectOutcome::err("boom"),
                    token: None,
                },
            },
            Event {
                seq: 8,
                cause: None,
                body: EventBody::EffectResult {
                    id: EffectId(9),
                    result: EffectOutcome::TimedOut,
                    token: None,
                },
            },
            Event {
                seq: 9,
                cause: None,
                // Some(token): exercise the §19e slice-2b-iii token codec on the arming frame.
                body: EventBody::TimerArmed {
                    id: EffectId(10),
                    deadline_ms: 999,
                    token: Some(b"timer-tok".to_vec()),
                },
            },
            Event {
                seq: 10,
                cause: None,
                body: EventBody::TimerFired {
                    id: EffectId(10),
                    fired_ms: 1000,
                    token: Some(b"timer-tok".to_vec()),
                },
            },
            Event {
                seq: 11,
                cause: None,
                // Some(token) on a denial too: a denied effect's continuation token rides the denial.
                body: EventBody::AuthzDenied {
                    id: EffectId(11),
                    reason: "no capability".into(),
                    token: Some(b"denied-tok".to_vec()),
                },
            },
            Event {
                seq: 12,
                cause: None,
                body: EventBody::Closed {
                    outcome: CloseOutcome::Success(Payload::Inline(vec![].into())),
                },
            },
            Event {
                // Both CloseOutcome arms in the frozen per-variant harness (fix-forward on #1938): the Failure
                // arm (its own reason string + fresh tag) is exercised by the frozen round-trip net.
                seq: 120,
                cause: None,
                body: EventBody::Closed {
                    outcome: CloseOutcome::Failure("goal abandoned: retries exhausted".to_string()),
                },
            },
            Event {
                seq: 13,
                cause: None,
                body: EventBody::FoldFailed {
                    reason: "wasm reducer trapped: unreachable".to_string(),
                    caused_event: Hash::of(b"the event whose fold failed"),
                },
            },
            // Spawned PRECEDES Terminated (seq 14 < 15): these vectors round-trip per-event independently,
            // but ordering the spawn-edge before the terminal marker avoids implying an impossible history
            // (a spawn AFTER the terminal tail — the "Terminated is the log tail" invariant forbids it).
            Event {
                seq: 14,
                cause: Some(Hash::of(b"spawn-cause")),
                body: EventBody::Spawned {
                    child_hash: Hash::of(b"child-genesis"),
                },
            },
            Event {
                seq: 15,
                cause: Some(Hash::of(b"terminate-cause")),
                body: EventBody::Terminated {
                    by: Hash::of(b"controller-session"),
                    reason: "operator kill".to_string(),
                },
            },
            // ChildCompleted, both CloseOutcome arms (self-close Success + terminate/failure), so both
            // codecs (event.rs + event_ast) round-trip the new variant across the exhaustive nets.
            Event {
                seq: 16,
                cause: Some(Hash::of(b"reap-cause")),
                body: EventBody::ChildCompleted {
                    child: Hash::of(b"child-genesis"),
                    outcome: CloseOutcome::Success(Payload::Inline(
                        b"child-result".to_vec().into(),
                    )),
                },
            },
            Event {
                seq: 17,
                cause: Some(Hash::of(b"reap-cause")),
                body: EventBody::ChildCompleted {
                    child: Hash::of(b"failed-child"),
                    outcome: CloseOutcome::Failure("child goal unreachable".to_string()),
                },
            },
            // A GAP-4 Checkpoint frame — exercises every CheckpointDescriptor field and both Some/None per
            // optional (across two obligations: a full effect obligation with all-Some + a timer obligation
            // with deadline/is_timer and the rest None), non-empty settled_exceptions + spawned, and a
            // Some(close_outcome), so the identity codec (and later event_ast) round-trip the new variant.
            Event {
                seq: 18,
                cause: None,
                body: EventBody::Checkpoint(CheckpointDescriptor {
                    kv_root: Hash::of(b"kv@18"),
                    next_effect_id: 42,
                    last_now: 1_000_000,
                    settled_watermark: 40,
                    settled_exceptions: vec![41, 43],
                    open: vec![
                        CheckpointObligation {
                            id: 42,
                            target: "https://ok.host/p".as_bytes().into(),
                            schema_hash: Some(crate::ast_marshal::builtin_effect_schema_hash(
                                &crate::effect::EffectKind::Http,
                            )),
                            token: Some(b"resume-tok".to_vec()),
                            deadline_ms: None,
                            dispatch_hash: Some(Hash::of(b"dispatch@42")),
                            is_timer: false,
                        },
                        CheckpointObligation {
                            id: 44,
                            target: b"".as_slice().into(),
                            schema_hash: None,
                            token: None,
                            deadline_ms: Some(999),
                            dispatch_hash: None,
                            is_timer: true,
                        },
                    ],
                    spawned: vec![Hash::of(b"child-a"), Hash::of(b"child-b")],
                    seeded_capabilities: true,
                    close_outcome: Some(CloseOutcome::Failure(
                        "checkpointed after failure".to_string(),
                    )),
                }),
            },
        ]
    }

    #[test]
    fn legacy_closed_payload_stream_decodes_as_success_unchanged() {
        // FROZEN-WIRE COMPAT (fix-forward on #1938, which shipped a wire-breaking wrapper tag): a Closed
        // stream written BEFORE the CloseOutcome change was push(7) + a bare Payload (its own inner tag
        // 0=Inline / 1=Blob). CloseOutcome::Success must encode BYTE-IDENTICALLY to that so an old log decodes
        // as Success — same bytes, same event hash. Build the legacy stream by hand + prove decode + re-encode.
        let payload = Payload::Inline(b"legacy-result".to_vec().into());
        let (seq, cause) = (5u64, Hash::of(b"parent-close"));
        let mut legacy = Vec::new();
        legacy.extend_from_slice(&seq.to_le_bytes());
        legacy.push(1); // cause present
        legacy.extend_from_slice(cause.as_bytes());
        legacy.push(7); // Closed body tag
        encode_payload(&payload, &mut legacy); // bare Payload, NO wrapper tag (the legacy shape)

        // 1. The legacy stream decodes to Closed{Success(the same payload)} — NOT a parse error.
        let (decoded, n) = Event::decode(&legacy).expect("legacy Closed stream still decodes");
        assert_eq!(n, legacy.len(), "consumes exactly the legacy bytes");
        assert_eq!(
            decoded.body,
            EventBody::Closed {
                outcome: CloseOutcome::Success(payload.clone()),
            },
            "a legacy Closed{{Payload}} stream decodes as Success (backward-compatible)"
        );

        // 2. The NEW encoder produces byte-identical output for that Success → old readers + event hashes match.
        let new_event = Event {
            seq,
            cause: Some(cause),
            body: EventBody::Closed {
                outcome: CloseOutcome::Success(payload),
            },
        };
        assert_eq!(
            new_event.encode(),
            legacy,
            "Success re-encodes byte-identically to the legacy Closed{{Payload}} wire (no wrapper tag)"
        );

        // 3. Failure takes a FRESH tag (2) a legacy Payload never led with (0/1) → no collision, round-trips.
        let fail = Event {
            seq: 6,
            cause: None,
            body: EventBody::Closed {
                outcome: CloseOutcome::Failure("boom".to_string()),
            },
        };
        let (rt, _) = Event::decode(&fail.encode()).unwrap();
        assert_eq!(rt, fail, "Failure round-trips on its fresh tag");
    }

    #[test]
    fn encode_decode_round_trips_every_variant() {
        for e in all_variants() {
            let bytes = e.encode();
            let (decoded, consumed) = Event::decode(&bytes).expect("decode");
            assert_eq!(decoded, e, "round-trip mismatch for {e:?}");
            assert_eq!(
                consumed,
                bytes.len(),
                "must consume exactly the encoded bytes"
            );
        }
    }

    #[test]
    fn decode_reports_offset_so_a_stream_can_be_walked() {
        // Two events concatenated (the on-disk log shape): decode the first, then the second from the
        // reported offset. This is what the durable-log reader will do.
        let events = all_variants();
        let mut stream = Vec::new();
        for e in &events[..2] {
            stream.extend_from_slice(&e.encode());
        }
        let (e0, n0) = Event::decode(&stream).unwrap();
        let (e1, _n1) = Event::decode(&stream[n0..]).unwrap();
        assert_eq!(e0, events[0]);
        assert_eq!(e1, events[1]);
    }

    #[test]
    fn truncated_input_errs_never_panics() {
        // A torn write at the tail: every proper prefix of a valid encoding must Err, not panic
        // (totality — the durable log must survive a partial final write).
        for e in all_variants() {
            let bytes = e.encode();
            for cut in 0..bytes.len() {
                let _ = Event::decode(&bytes[..cut]); // must not panic
            }
        }
    }

    #[test]
    fn corrupt_tag_is_rejected() {
        let mut bytes = genesis().encode();
        // seq(8) + cause-tag(1) = offset 9 is the body variant tag; set it to an unknown value.
        bytes[9] = 250;
        assert!(matches!(
            Event::decode(&bytes),
            Err(DecodeError::BadTag { .. })
        ));
    }

    #[test]
    fn oversized_length_prefix_errs_never_panics() {
        // PR#990 finding #3: a length prefix from the untrusted log that is enormous (here u64::MAX)
        // must fail cleanly (BadLength on 32-bit where it can't fit usize; Truncated on 64-bit where it
        // exceeds the remaining bytes) — NEVER panic or wrap-truncate into a mis-parse. Build an
        // Inbound whose content_type.family length prefix is u64::MAX.
        // Inbound encoding: seq(8) + cause-tag(1=0) + body-tag(1) + family-len(8) + ...
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1u64.to_le_bytes()); // seq
        bytes.push(0); // cause: None
        bytes.push(1); // body tag = Inbound
        bytes.extend_from_slice(&u64::MAX.to_le_bytes()); // family length = absurd
        bytes.extend_from_slice(b"anything");
        match Event::decode(&bytes) {
            Err(DecodeError::BadLength) | Err(DecodeError::Truncated) => {}
            other => panic!("expected BadLength/Truncated for an oversized length, got {other:?}"),
        }
    }
}
