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
use cdz_kernel::effect::{effect_ct, ResourcePredicate};
use cdz_kernel::event::EventBody;
use cdz_kernel::executor::{CompositeExecutor, Executor};
use cdz_kernel::hash::Hash;
use cdz_kernel::kernel::{KernelError, Session};
use cdz_kernel::reducer::Reducer;
use std::collections::HashMap;

/// The host-emitted GENESIS-SETUP content-type families — the guest↔host bootstrap contract agreed with
/// v-harness-bootstrap (whose `reducer_genesis.cdz` folds each into session KV, requesting no effects). After
/// a session's reducer-hash genesis, the host seed path delivers these as ORDINARY early inbound events
/// (content-as-events, design §3 — NOT a side channel); the reducer recognizes the family and folds the
/// payload to a well-known KV key. Same `family` namespacing as the effect content-types
/// ([`cdz_kernel::effect::effect_ct`]).
pub mod genesis_ct {
    /// The trust-ROOT identity — arrives as this event's PAYLOAD, never baked (operator seq-129). Reducer
    /// folds to KV `bootstrap/root-identity`.
    pub const ROOT: &str = "genesis/root";
    /// The authorizer component's HASH pointer (§20b install-authorizer-by-hash). Reducer folds to KV
    /// `bootstrap/authorizer-hash`; the host's reload-policy path later resolves it to install the real authorizer.
    pub const AUTHORIZER: &str = "genesis/authorizer";
    /// Free-form bootstrap CONTEXT. Reducer folds to KV `bootstrap/context`.
    pub const CONTEXT: &str = "genesis/context";
    /// The content-type version stamped on genesis-setup events (v1 of the bootstrap contract).
    pub const VERSION: u32 = 1;

    /// The well-known session-KV KEYS the genesis reducer folds each setup family's payload into (the reducer
    /// stores the event payload VERBATIM — v-harness-bootstrap confirmed, so the byte-form is the host's choice
    /// on both write and read). The host's genesis-completion glue reads these to resolve the recorded pointers.
    pub const KV_ROOT_IDENTITY: &[u8] = b"bootstrap/root-identity";
    /// KV key holding the authorizer component's content hash (raw 32 bytes) — the host resolves this to the
    /// policy component + installs the real authorizer.
    pub const KV_AUTHORIZER_HASH: &[u8] = b"bootstrap/authorizer-hash";
    /// KV key holding the free-form bootstrap context blob.
    pub const KV_CONTEXT: &[u8] = b"bootstrap/context";
}

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

/// Mint a fresh SPAWN NONCE for a root session's genesis (§lifecycle I2a) — 32 OS-random bytes hashed into
/// a [`Hash`]. This is the host-supplied entropy that makes `genesis_hash` (= the SessionId) per-session
/// UNIQUE: two sessions over the same reducer get different nonces → different ids. We `Hash::of` the
/// random bytes rather than `Hash::from_bytes` them so the value is a real content hash (blake3 domain),
/// not raw bytes coerced into the `Hash` type — the nonce is a HASH of entropy, uniformly with every other
/// `Hash` in the system. `getrandom` panicking is not survivable (no entropy source = we can't safely mint
/// a unique id), so a failure is a hard error, not a silent weak-nonce fallback.
pub(crate) fn mint_spawn_nonce() -> Hash {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).expect("OS entropy (getrandom) for a session spawn nonce");
    Hash::of(&bytes)
}

/// A session's DIRECT spawn-children as [`SessionId`]s (§lifecycle I6): each `Spawned` edge records the
/// child's genesis `Hash`, and a child's SessionId IS its genesis-hash-hex, so map hash → hex → SessionId.
fn child_ids(s: &HostedSession) -> Vec<SessionId> {
    s.spawned_children()
        .iter()
        .map(|h| SessionId::new(h.to_hex()))
        .collect()
}

/// One running agent: the kernel `Session` plus the collaborators that drive its loop. The host owns all
/// of them for the session's lifetime, so a registry can drive the session by id (the kernel's `deliver`
/// borrows reducer/authz/executor per call; bundling them here is what lets the host re-supply them).
pub struct HostedSession {
    session: Session,
    reducer: Box<dyn Reducer>,
    authz: Box<dyn Authorize>,
    executor: CompositeExecutor,
    /// SUSPENDED — a HOST-SCHEDULER bit (§lifecycle I4), NOT kernel/session state: when true the host stops
    /// SCHEDULING this session's ticks (the loop holds its inbound instead of delivering; timers don't fire
    /// for it), but the durable log + KV are UNTOUCHED — resume just re-enables scheduling and the held
    /// inbound replays. Kept out of the kernel entirely (v-agent-harness refinement): "suspended" is a
    /// transient scheduler state, not a durable session fact, so it doesn't touch the log or survive a
    /// recovery (a recovered session starts schedulable — a supervisor re-suspends if it wants). Distinct
    /// from TERMINATED ([`is_terminated`](Self::is_terminated)), which IS a durable kernel marker.
    suspended: bool,
}

impl HostedSession {
    /// Start a fresh session from a genesis reducer hash, with its executor set + authorizer. The
    /// `reducer` drives folds; `executor` (a by-family-string [`CompositeExecutor`]) performs authorized effects;
    /// `authz` gates them (SEC-F1). This is the assembly point — real executors (Now/Model/Http) go into
    /// `executor`, a real policy into `authz`.
    ///
    /// `reducer` is a `Box<dyn Reducer>`: a pure-Rust reducer is passed as
    /// `Box::new(my_reducer)`, a wasm reducer as `Box::new(AsyncComponentReducer::…)`.
    ///
    /// A fresh, OS-random SPAWN NONCE is minted here and hashed into the seq-0 Genesis event (§lifecycle
    /// I2a): `Session::genesis(reducer, spawn_nonce)` makes `genesis_hash` — the host's SessionId primitive
    /// — per-session UNIQUE, so two sessions over the SAME reducer no longer collide on their id. The kernel
    /// stays entropy-free (§9c): the host is the entropy source, the nonce rides the durable log, and
    /// recovery reads it back from the log (never re-mints) so a recovered session keeps its id. This is a
    /// ROOT session (`parent = None`); the `lifecycle/spawn` child path (I3) uses
    /// [`Session::genesis_spawned`] with the parent's genesis hash instead.
    pub fn genesis(
        reducer_hash: Hash,
        reducer: Box<dyn Reducer>,
        authz: Box<dyn Authorize>,
        executor: CompositeExecutor,
    ) -> Self {
        Self::genesis_with_nonce(reducer_hash, mint_spawn_nonce(), reducer, authz, executor)
    }

    /// Like [`genesis`](Self::genesis) but the caller SUPPLIES the root `spawn_nonce` instead of minting a
    /// fresh one. Useful when a caller must know the resulting `genesis_hash` (= SessionId) BEFORE building
    /// the session — e.g. to wire a self-referencing collaborator (a lifecycle executor whose `owner` is this
    /// session's own id): derive the id via [`Session::derive_genesis_hash`]`(reducer, nonce, None)` first,
    /// then build with the SAME nonce so the actual genesis-hash matches. `genesis` (mint-internally) stays
    /// the common path.
    pub fn genesis_with_nonce(
        reducer_hash: Hash,
        spawn_nonce: Hash,
        reducer: Box<dyn Reducer>,
        authz: Box<dyn Authorize>,
        executor: CompositeExecutor,
    ) -> Self {
        HostedSession {
            session: Session::genesis(reducer_hash, spawn_nonce),
            reducer,
            authz,
            executor,
            suspended: false,
        }
    }

    /// Start a SPAWNED CHILD session (§lifecycle I3): like [`genesis`](Self::genesis) but the seq-0 Genesis
    /// event carries `parent = Some(parent_genesis_hash)` (the spawning session's id = its genesis hash) so
    /// the child's own `genesis_hash` — and thus its SessionId — is PROVENANCE-DEPENDENT (it self-certifies
    /// which session spawned it), per [`Session::genesis_spawned`]. A fresh OS-random spawn nonce is still
    /// minted here (uniqueness even among same-reducer + same-parent children). The durable parent→child
    /// EDGE (the supervision tree the Cedar `DescendantOf` authority walks) is recorded separately on the
    /// PARENT's log via [`AgentHost::spawn_child`], which drives this + [`Session::record_spawn`].
    pub fn genesis_spawned(
        reducer_hash: Hash,
        parent_genesis_hash: Hash,
        reducer: Box<dyn Reducer>,
        authz: Box<dyn Authorize>,
        executor: CompositeExecutor,
    ) -> Self {
        Self::genesis_spawned_with_nonce(
            reducer_hash,
            mint_spawn_nonce(),
            parent_genesis_hash,
            reducer,
            authz,
            executor,
        )
    }

    /// Like [`genesis_spawned`](Self::genesis_spawned) but the caller SUPPLIES the `spawn_nonce` instead of
    /// minting a fresh one (§lifecycle I3 spawn-executor). The `lifecycle/spawn` executor needs this: to
    /// return the child's `SessionId` (= its genesis hash) SYNCHRONOUSLY as the effect result while the loop
    /// registers the child AFTER `deliver` (defer-to-loop), the executor PRE-COMPUTES the child hash via
    /// [`Session::derive_genesis_hash`](cdz_kernel::kernel::Session::derive_genesis_hash)`(reducer, nonce,
    /// Some(parent))` — which only matches what the loop registers if the loop builds the child with the
    /// SAME nonce (not a re-mint). So the executor mints ONCE, carries the nonce to the loop, and the loop
    /// calls THIS. `genesis_spawned` (the mint-internally form) stays for callers that don't need a
    /// pre-known id.
    pub fn genesis_spawned_with_nonce(
        reducer_hash: Hash,
        spawn_nonce: Hash,
        parent_genesis_hash: Hash,
        reducer: Box<dyn Reducer>,
        authz: Box<dyn Authorize>,
        executor: CompositeExecutor,
    ) -> Self {
        HostedSession {
            session: Session::genesis_spawned(reducer_hash, spawn_nonce, Some(parent_genesis_hash)),
            reducer,
            authz,
            executor,
            suspended: false,
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

    /// Attach a durable [`LogSink`](cdz_kernel::log_store::LogSink) as this session's write-through target —
    /// a builder over [`genesis`](Self::genesis). Each event the session appends is also persisted through
    /// the sink (the kernel LATCHES a sink append failure + refuses to route, §16c-S1 — so a durable
    /// dispatch is never routed on an un-persisted event). ADDITIVE: a session with no sink keeps its
    /// in-memory log only (dev/test); the deployed daemon attaches a per-session
    /// [`LogStore`](cdz_kernel::log_store::LogStore) (a single-file durable log) when `[log].backend = file`.
    ///
    /// Per-session by value (like [`with_name_store`](Self::with_name_store)): the caller opens the sink for
    /// this session (e.g. `LogStore::open(dir/<id>.log)`) and hands it over; the session owns it for its
    /// lifetime.
    pub fn with_sink(mut self, sink: Box<dyn cdz_kernel::log_store::LogSink>) -> Self {
        self.session.attach_sink(sink);
        self
    }

    /// Register (or replace) a by-family effect executor on this LIVE session — the MECHANISM axis of a
    /// mid-session capability change (host-capability-discovery I6a). Adding an executor for a family the
    /// session couldn't perform before makes that family's manifest entry flip `Absent`→(policy-decided);
    /// re-registering a family swaps its executor. This is the host-side trigger the reactive
    /// capabilities-changed push (I6b) fires on: after calling it, [`push_capabilities_changed`] recomputes
    /// the manifest + pushes iff it actually changed.
    ///
    /// [`push_capabilities_changed`]: HostedSession::push_capabilities_changed
    ///
    /// (The kernel's [`CompositeExecutor`] is builder-shaped — `with_effect` consumes+returns — so this
    /// takes the executor by value via `mem::take` + rebuild, keeping the mutation on the owned field.)
    pub fn add_executor(&mut self, family: impl Into<String>, executor: Box<dyn Executor>) {
        let composite = std::mem::take(&mut self.executor);
        self.executor = composite.with_effect(family, executor);
    }

    /// Swap this LIVE session's authorizer — the POLICY axis of a mid-session capability change (I6a). A
    /// new policy (e.g. a broadened/tightened grant, or one loaded from a §4c policy-pointer `store/set`)
    /// changes which effects authorize, so a family's manifest entry can flip `Denied`↔`Granted` with the
    /// same executor set. Pair with [`push_capabilities_changed`](Self::push_capabilities_changed) to push
    /// the resulting delta.
    pub fn set_authorizer(&mut self, authz: Box<dyn Authorize>) {
        self.authz = authz;
    }

    /// Recompute this session's capability manifest against its CURRENT (post-mutation) authorizer +
    /// executor set and, IFF it moved vs the manifest the guest last saw, fold a `capabilities-changed`
    /// EffectResult back to the reducer — the reactive push (host-capability-discovery I6b). This is the
    /// host trigger: call it after an [`add_executor`](Self::add_executor) / [`set_authorizer`](Self::set_authorizer)
    /// mutation (or a §4c policy-pointer change) to notify a capability-aware agent that its usable surface
    /// changed mid-session.
    ///
    /// A NO-OP when nothing changed (the "delivered only to sessions whose manifest actually changed" gate,
    /// which also gives free coalescing: call it once per settle point after a burst of mutations, not
    /// per-mutation, and a net-zero burst pushes nothing). The push reuses the SAME manifest shape as seed
    /// (I5) / query (I4), so a capability-aware reducer decodes ONE shape however the manifest arrived.
    /// Returns any [`ControlEffect`](cdz_kernel::effect::ControlEffect)s the fold surfaced (usually none).
    ///
    /// Baseline note (from the kernel seam): the last-known manifest it diffs against is EPHEMERAL host-side
    /// state repopulated by projection (seed/query/push), NOT replay-rebuilt — so a freshly RECOVERED
    /// session with no baseline pushes its current manifest on the first call (correct: the recovered guest
    /// re-learns its surface). Re-seed after recover if you want to suppress that until a real change.
    pub async fn push_capabilities_changed(&mut self) -> Vec<cdz_kernel::effect::ControlEffect> {
        self.session
            .push_capabilities_changed(&*self.reducer, &*self.authz, &mut self.executor)
            .await
    }

    /// LIVE-SWAP this session's policy from a Cedar policy-component blob, then push the resulting
    /// capability change — the §20b policy-referenced-by-mutable-name close-out. A privileged admin
    /// `store/set`s [`POLICY_CURRENT`](cdz_kernel::name_store::NameStore::POLICY_CURRENT) → a policy blob
    /// hash (write-gated by a `system/` grant — the anti-hijack property); the host resolves that pointer,
    /// blob-gets the component `bytes`, and calls this to rebuild the authorizer + notify the agent.
    ///
    /// It lifts `bytes` into a [`ComponentAuthorizer`](cdz_kernel::wasm_host::ComponentAuthorizer) (the
    /// lifted Cedar policy), installs it via [`set_authorizer`](Self::set_authorizer), then calls
    /// [`push_capabilities_changed`](Self::push_capabilities_changed) — so a policy swap that widened or
    /// tightened a grant folds a `capabilities-changed` to the agent (a no-op if grant-states didn't move).
    /// Returns the pushed [`ControlEffect`](cdz_kernel::effect::ControlEffect) list (usually empty), or `Err`
    /// if the bytes aren't a valid policy component (the swap is then NOT applied — the old policy stays).
    ///
    /// `principal` is the agent's authz principal (e.g. `"agent://<id>"`), the same value the session's
    /// original authorizer was built with. The host owns resolving POLICY_CURRENT + the blob fetch (it holds
    /// the blob store); this method takes the already-fetched bytes so `HostedSession` stays blob-store-free.
    pub async fn reload_policy_from_component_bytes(
        &mut self,
        bytes: &[u8],
        principal: impl Into<String>,
    ) -> Result<Vec<cdz_kernel::effect::ControlEffect>, String> {
        let authz = cdz_kernel::wasm_host::ComponentAuthorizer::from_policy_bytes(bytes, principal)
            .map_err(|e| format!("policy component did not lift into an authorizer: {e:?}"))?;
        self.set_authorizer(Box::new(authz));
        Ok(self.push_capabilities_changed().await)
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

    /// Seed the GENESIS-SETUP events into a freshly-`genesis`'d session — the host side of the bootstrap
    /// ceremony (contract with v-harness-bootstrap). Delivers, in order, a [`genesis/root`](genesis_ct::ROOT)
    /// event carrying the established trust-root identity, an optional [`genesis/authorizer`](genesis_ct::AUTHORIZER)
    /// event carrying the authorizer component's hash, and an optional [`genesis/context`](genesis_ct::CONTEXT)
    /// event — each as an ordinary early inbound [`deliver`](Self::deliver) (content-as-events, §3), which the
    /// genesis reducer folds into session KV (`bootstrap/root-identity` / `bootstrap/authorizer-hash` /
    /// `bootstrap/context`). These setup events request NO effects, so the session's deny-all v0 authorizer
    /// doesn't gate them; the recorded `authorizer-hash` is the pointer a later reload-policy step resolves to
    /// install the real authorizer.
    ///
    /// Stops at the FIRST delivery error (a genesis fold failure is fatal to the ceremony — the caller surfaces
    /// it rather than booting a half-seeded session). `authorizer_hash`/`context` are optional (a minimal boot
    /// seeds only the root).
    pub async fn seed_genesis(
        &mut self,
        root_identity: &[u8],
        authorizer_hash: Option<&[u8]>,
        context: Option<&[u8]>,
    ) -> Result<(), KernelError> {
        self.deliver(genesis_event(genesis_ct::ROOT, root_identity), None)
            .await?;
        if let Some(h) = authorizer_hash {
            self.deliver(genesis_event(genesis_ct::AUTHORIZER, h), None)
                .await?;
        }
        if let Some(c) = context {
            self.deliver(genesis_event(genesis_ct::CONTEXT, c), None)
                .await?;
        }
        Ok(())
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

    /// This session's GENESIS HASH — the content hash of its genesis event ([`Session::genesis_hash`], =
    /// `log[0].hash()`). This is the host's canonical SessionId primitive: a SPAWNED child's id IS its
    /// genesis-hash-hex (the operator ruling — provenance-derived + self-certifying), which the naming layer
    /// resolves a name to (name → genesis-hash = the routable id, no separate hash→id map needed).
    ///
    /// ⚠ NOT a GLOBAL identity — a [`SessionId`] is OPAQUE host-assigned metadata: a spawned child gets
    /// genesis-hash-hex, but a root / named session may carry a VANITY id (e.g. `"concierge"`). So code
    /// holding a genesis `Hash` (e.g. [`Session::parent`]) must resolve it to a registry entry via
    /// [`AgentHost::session_id_by_genesis_hash`] (matches on `genesis_hash()`), NEVER by assuming
    /// `hex(hash) == id` globally — that assumption is exactly what silently dropped a `ChildExited` to a
    /// vanity-id supervisor (PR #2481 c1). `hex(this)` is a valid id to ASSIGN a fresh session, not a
    /// guarantee of how an existing one is keyed.
    ///
    /// PER-SESSION UNIQUE (§lifecycle I2a): the genesis event carries a fresh OS-random SPAWN NONCE
    /// (minted by [`mint_spawn_nonce`] at [`HostedSession::genesis`], or the parent's provenance at
    /// `Session::genesis_spawned` for a spawned child), hashed into the seq-0 event alongside the reducer
    /// hash. So two sessions over the SAME reducer get DIFFERENT genesis_hashes → distinct SessionIds — no
    /// registry-key collision (see `two_same_reducer_sessions_get_distinct_genesis_hashes_uniqueness_gap_closed`).
    /// The nonce is host-supplied (the kernel stays entropy-free, §9c) and rides the durable log, so a
    /// recovered/replayed session reconstructs the SAME genesis_hash from its log — the id is stable across
    /// recovery, never re-minted.
    pub fn genesis_hash(&self) -> Hash {
        self.session.genesis_hash()
    }

    /// TERMINATE this session (§lifecycle I5): install the durable [`EventBody::Terminated`] marker as the
    /// log tail via the kernel's fold-free [`Session::terminate`] seam. `by` is the terminating controller's
    /// identity (its genesis hash = its SessionId); `reason` is a diagnostic string. Returns the marker's
    /// event hash (for cause-linking / logging). AFTER this the session is [`is_terminated`](Self::is_terminated)
    /// and the kernel refuses every further fold ([`KernelError::FoldRefused`]) — a frozen, queryable
    /// tombstone (log + KV retained). The host's `lifecycle/terminate` executor drives this, then REMOVES the
    /// session from the [`AgentHost`] registry (see [`AgentHost::terminate`]); an in-flight `Emit` to the
    /// now-terminated/absent target bounces as a `delivery-failure` at the loop routing arm (the I5 bounce).
    ///
    /// IDEMPOTENT-BY-REJECTION: terminating an already-terminated session returns [`KernelError::FoldRefused`]
    /// (the kernel's contract) — never a second marker. Terminal: there is no un-terminate.
    pub async fn terminate(&mut self, by: Hash, reason: String) -> Result<Hash, KernelError> {
        self.session.terminate(by, reason).await
    }

    /// Is this session TERMINATED — its log tail is the durable [`EventBody::Terminated`] marker (§lifecycle
    /// I1). Delegates to [`Session::is_terminated`]; the host consults it to distinguish a terminated target
    /// (bounce an `Emit` as `delivery-failure`) from a live one, and to guard against re-driving a tombstone.
    pub fn is_terminated(&self) -> bool {
        self.session.is_terminated()
    }

    /// SUSPEND this session (§lifecycle I4) — set the host-scheduler `suspended` bit so the loop stops
    /// scheduling its ticks (holds inbound, skips its timers). NO log/KV mutation (suspend is transient
    /// scheduler state, not a durable fact). Idempotent: suspending an already-suspended session is a no-op.
    /// A TERMINATED session can't be meaningfully suspended, but this doesn't guard it (terminate already
    /// froze the log + the loop won't deliver to it); the host's `AgentHost::suspend` is the driven entry.
    pub fn suspend(&mut self) {
        self.suspended = true;
    }

    /// RESUME this session (§lifecycle I4) — clear the `suspended` bit so the loop schedules it again; any
    /// inbound the loop held during suspension replays. NO log/KV mutation. Idempotent.
    pub fn resume(&mut self) {
        self.suspended = false;
    }

    /// Is this session SUSPENDED (host-scheduler bit, §lifecycle I4)? The loop checks this before delivering
    /// an inbound / firing a timer — a suspended session's inbound is HELD (re-queued), not delivered or
    /// dropped. Orthogonal to [`is_terminated`](Self::is_terminated) (a durable kernel marker).
    pub fn is_suspended(&self) -> bool {
        self.suspended
    }

    /// Record a durable parent→child EDGE on this (parent) session's log (§lifecycle I2b/I3): appends a
    /// `Spawned{child_hash}` event via the kernel's fold-free [`Session::record_spawn`] seam. Returns the edge
    /// event hash; refused ([`KernelError::FoldRefused`]) if this session is terminated (a tombstone can't
    /// spawn). Driven by [`AgentHost::spawn_child`] after it builds the child.
    pub async fn record_spawn(&mut self, child_hash: Hash) -> Result<Hash, KernelError> {
        self.session.record_spawn(child_hash).await
    }

    /// This session's direct spawn-children (§lifecycle I2b), in spawn order — the `child_hash`es of the
    /// `Spawned` edges on its log. Delegates to [`Session::spawned_children`]; the Cedar `DescendantOf`
    /// authority (I6) walks these transitively to decide who may terminate/suspend whom.
    pub fn spawned_children(&self) -> Vec<Hash> {
        self.session.spawned_children()
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

/// Build a genesis-setup inbound event: an [`EventBody::Inbound`] with the given `family` content-type (at
/// [`genesis_ct::VERSION`]) carrying `payload` inline. The `payload` bytes are the setup value the genesis
/// reducer folds into KV (e.g. the root identity / the authorizer hash / the context blob).
fn genesis_event(family: &'static str, payload: &[u8]) -> EventBody {
    EventBody::Inbound {
        content_type: cdz_kernel::event::ContentType {
            // The genesis families are `&'static str` consts → a borrowed Cow, no allocation.
            family: std::borrow::Cow::Borrowed(family),
            version: genesis_ct::VERSION,
        },
        payload: cdz_kernel::effect::Payload::Inline(payload.to_vec().into()),
    }
}

/// The host: a registry of running agent sessions keyed by [`SessionId`]. Owns each [`HostedSession`],
/// routes inbound events to the right one, and is the object a `session-status <id>` query reads.
///
/// Constructed via [`new`](Self::new) / [`with_canonical_store`](Self::with_canonical_store) (each creates
/// the metrics registry). `Default` delegates to `new` (it can't be derived — the metrics registry is a
/// required collaborator, not a `Default` field — so the impl forwards to `new`).
pub struct AgentHost {
    sessions: HashMap<SessionId, HostedSession>,
    /// The §4c v0.3 canonical shared name store, if this host is share-backed (see
    /// [`AgentHost::with_canonical_store`]). `None` = share-less host. Held BY VALUE: each session gets a
    /// replay-copy at spawn and folds its appends back after each turn — no shared handle. (Type:
    /// [`cdz_kernel::name_store::NameStore`].)
    canonical: Option<cdz_kernel::name_store::NameStore>,
    /// The s2n-quic-dc-metrics registry the host records into — the recorder surface an export backend drains.
    /// Held so `metrics` (and the executor set's `EffectMetrics`) register into ONE registry, and so
    /// [`AgentHost::registry`] can hand it to the exporter.
    registry: crate::metrics::Registry,
    /// The host's metric surface — registry [`Counter`]s bumped at the session-lifecycle + per-turn
    /// boundaries (spawn/remove/deliver), registered from [`registry`](Self::registry).
    metrics: crate::metrics::HostMetrics,
}

/// A by-value copy of a canonical [`NameStore`](cdz_kernel::name_store::NameStore) for a freshly-spawned session — replays the canonical
/// store's full set-event stream into a fresh store (§4c v0.3 spawn step). `to_set_entries` +
/// `replay_set_entries` are total over a valid store (single-writer-per-name → no `Unscoped` name in the
/// stream), so this can't fail in practice; a defensive `expect` documents that invariant rather than
/// threading a Result through spawn (which the caller can't meaningfully recover from).
fn replay_of(canonical: &cdz_kernel::name_store::NameStore) -> cdz_kernel::name_store::NameStore {
    let entries = canonical.to_set_entries();
    cdz_kernel::name_store::NameStore::replay_set_entries(
        entries.iter().map(|(n, h)| (n.as_str(), *h)),
    )
    .expect("a canonical store holds only scoped names, so its replay is total")
}

impl Default for AgentHost {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentHost {
    pub fn new() -> Self {
        let registry = crate::metrics::Registry::new();
        let metrics = crate::metrics::HostMetrics::new(&registry);
        AgentHost {
            sessions: HashMap::new(),
            canonical: None,
            registry,
            metrics,
        }
    }

    /// Enable the §4c v0.3 SHARED name store — a single host-owned canonical [`NameStore`](cdz_kernel::name_store::NameStore) that gives LIVE
    /// cross-session visibility of published pointers, replacing the per-hand-off export/replay bridge.
    /// Opt-in: a host built with [`new`](Self::new) stays share-less and every session keeps whatever store
    /// (or none) it was spawned with.
    ///
    /// Lifecycle (single-writer-per-name, conflict-free): the host holds ONE canonical store; on
    /// [`spawn`](Self::spawn) a session gets a by-VALUE copy of it (a replay of `canonical.to_set_entries()`),
    /// so it's born seeing everyone's published pointers; after each [`deliver`](Self::deliver) turn the host
    /// folds that session's new writes back with `canonical.merge_appends_from(session.name_store())`. No
    /// shared handle / interior mutability — by-value copies + a reconcile, composing with the per-session
    /// [`HostedSession::with_name_store`] seam.
    pub fn with_canonical_store(canonical: cdz_kernel::name_store::NameStore) -> Self {
        let registry = crate::metrics::Registry::new();
        let metrics = crate::metrics::HostMetrics::new(&registry);
        AgentHost {
            sessions: HashMap::new(),
            canonical: Some(canonical),
            registry,
            metrics,
        }
    }

    /// Borrow the host-owned CANONICAL shared name store (`None` for a share-less host). The read-back dual
    /// of [`with_canonical_store`](Self::with_canonical_store): a driver observes the shared directory the
    /// host maintains (group memberships after death-retract, published pointers), e.g. to `resolve_all` a
    /// group post-eviction. Read-only — the host owns the mutation policy (session-write fold-back +
    /// §I5 death-retract).
    pub fn canonical_store(&self) -> Option<&cdz_kernel::name_store::NameStore> {
        self.canonical.as_ref()
    }

    /// Register a new running session under `id`. Returns the id back for convenience. If `id` already
    /// exists it is REPLACED (the caller chose to restart it) — the old session is dropped; a caller that
    /// wants collision-detection checks [`AgentHost::contains`] first.
    ///
    /// When the host has a canonical shared store (see [`with_canonical_store`](Self::with_canonical_store)),
    /// the session is born with a by-value replay of it — so a freshly-spawned agent already sees every
    /// pointer other sessions have published (and the host folds this session's new writes back after each
    /// turn). This REPLACES any store the session was built with (via
    /// [`HostedSession::with_name_store`]): a canonical-backed host is the single source of the shared name
    /// space, so it attaches the canonical replay unconditionally. On a share-less host (plain
    /// [`new`](Self::new)) the session keeps whatever store (or none) it was spawned with.
    pub fn spawn(&mut self, id: SessionId, session: HostedSession) -> SessionId {
        let session = match &self.canonical {
            Some(canonical) => session.with_name_store(replay_of(canonical)),
            None => session,
        };
        let replaced = self.sessions.insert(id.clone(), session).is_some();
        self.metrics.record_session_installed();
        // A spawn onto an existing id REPLACES (drops) the old session without a `remove` call — count that
        // implicit drop as a removal too, so `sessions_live` (installed − removed) stays accurate.
        if replaced {
            self.metrics.record_session_removed();
        }
        // Trace at the same boundary the metric records — a structured event (side-channel, no control-flow
        // dependence). `replaced` distinguishes a fresh install from a restart.
        tracing::info!(
            target: "cdz_agent_host::session",
            session_id = id.as_str(),
            replaced,
            "session installed"
        );
        id
    }

    /// SPAWN A CHILD session under `parent` (§lifecycle I3): the `lifecycle/spawn` effect's registry side.
    /// Builds a child [`HostedSession`] from `reducer_hash` + its executor/authz with parent-provenance
    /// ([`HostedSession::genesis_spawned`]), derives the child's `SessionId` from its genesis hash
    /// (`hex(genesis_hash)` — a SPAWNED child's id IS its genesis-hash-hex per the operator ruling, so the id
    /// is provenance- + nonce-unique; note this is the SPAWNED-child rule, NOT a global one — a root/named
    /// session may carry an opaque vanity id, see [`AgentHost::session_id_by_genesis_hash`]), inserts it into
    /// the registry, and records the durable parent→child EDGE on the PARENT's
    /// log ([`Session::record_spawn`]) — the supervision tree the Cedar `DescendantOf` authority (I6) walks.
    ///
    /// Returns:
    /// - `Some(Ok(child_id))` — spawned; the child is registered under `child_id = hex(child genesis_hash)`
    ///   and the parent's log carries the `Spawned{child_hash}` edge.
    /// - `Some(Err(FoldRefused))` — the parent is TERMINATED (its log refuses the `record_spawn` append); the
    ///   child is NOT registered (we record the edge FIRST so a terminated parent can't spawn — no orphan).
    /// - `None` — no such `parent` id (a robust host doesn't spawn under a phantom parent).
    ///
    /// Records the edge on the parent BEFORE inserting the child: if the parent is terminated the whole spawn
    /// is refused with nothing registered (atomic-ish — no dangling child whose parent rejected the edge).
    pub async fn spawn_child(
        &mut self,
        parent: &SessionId,
        reducer_hash: Hash,
        reducer: Box<dyn Reducer>,
        authz: Box<dyn Authorize>,
        executor: CompositeExecutor,
    ) -> Option<Result<SessionId, KernelError>> {
        self.spawn_child_with_nonce(
            parent,
            reducer_hash,
            mint_spawn_nonce(),
            reducer,
            authz,
            executor,
        )
        .await
    }

    /// Like [`spawn_child`](Self::spawn_child) but the caller SUPPLIES the `spawn_nonce` (§lifecycle I3
    /// spawn-executor). The `lifecycle/spawn` executor pre-computes the child's genesis hash from
    /// `(reducer_hash, spawn_nonce, Some(parent_genesis))` via
    /// [`Session::derive_genesis_hash`](cdz_kernel::kernel::Session::derive_genesis_hash) to return the child
    /// `SessionId` synchronously; the loop then calls THIS with the SAME nonce so the registered child's id
    /// matches the pre-computed one BYTE-FOR-BYTE (a re-mint would diverge). Same edge-first / terminated-
    /// parent-refused / phantom-parent-None contract as `spawn_child`.
    pub async fn spawn_child_with_nonce(
        &mut self,
        parent: &SessionId,
        reducer_hash: Hash,
        spawn_nonce: Hash,
        reducer: Box<dyn Reducer>,
        authz: Box<dyn Authorize>,
        executor: CompositeExecutor,
    ) -> Option<Result<SessionId, KernelError>> {
        // Build the child first (pure construction, no registry mutation) so we know its genesis hash = its
        // id, which is what the parent's edge records. The supplied nonce makes the id match a caller's
        // pre-computation (the spawn executor's derive_genesis_hash).
        let parent_genesis = self.sessions.get(parent)?.genesis_hash();
        let child = HostedSession::genesis_spawned_with_nonce(
            reducer_hash,
            spawn_nonce,
            parent_genesis,
            reducer,
            authz,
            executor,
        );
        self.spawn_child_prebuilt_with_nonce(parent, reducer_hash, spawn_nonce, child)
            .await
    }

    /// Register an ALREADY-BUILT spawned child under `parent` (§lifecycle I3 loop-apply). Same edge-first /
    /// terminated-parent-refused / phantom-parent-None contract as [`spawn_child_with_nonce`], but the caller
    /// (the loop, via the session factory's `build_spawned`) already materialized the child `HostedSession`
    /// — so this only records the edge + registers it, no rebuild. `reducer_hash`/`spawn_nonce` are accepted
    /// for signature symmetry + a debug check that `child`'s id is the expected provenance-derived one (the
    /// factory built it with the same triple, so they agree).
    pub async fn spawn_child_prebuilt_with_nonce(
        &mut self,
        parent: &SessionId,
        reducer_hash: Hash,
        spawn_nonce: Hash,
        child: HostedSession,
    ) -> Option<Result<SessionId, KernelError>> {
        let parent_genesis = self.sessions.get(parent)?.genesis_hash();
        let child_hash = child.genesis_hash();
        // The pre-built child MUST carry the same provenance the caller pre-computed from — a mismatch means
        // the factory built with a different reducer/nonce/parent than the op recorded (a wiring bug).
        debug_assert_eq!(
            child_hash,
            Session::derive_genesis_hash(reducer_hash, spawn_nonce, Some(parent_genesis)),
            "pre-built child's genesis hash must match its (reducer, nonce, parent) provenance"
        );
        let child_id = SessionId::new(child_hash.to_hex());

        // Record the durable parent→child edge FIRST: a terminated parent refuses the append (FoldRefused),
        // and we then register NOTHING — so a terminated session can never spawn a live orphan.
        let parent_session = self.sessions.get_mut(parent)?;
        if let Err(e) = parent_session.record_spawn(child_hash).await {
            return Some(Err(e));
        }
        // Edge recorded → register the child (reuses `spawn`: canonical-store replay + metrics + trace).
        self.spawn(child_id.clone(), child);
        Some(Ok(child_id))
    }

    /// Is a session registered under this id?
    pub fn contains(&self, id: &SessionId) -> bool {
        self.sessions.contains_key(id)
    }

    /// SUSPEND a registered session by id (§lifecycle I4): flip its host-scheduler `suspended` bit so the
    /// loop holds its inbound / skips its timers (no log mutation — suspend is transient scheduler state, so
    /// this stays synchronous, unlike terminate which appends a durable marker). Returns `true` if the session
    /// exists (suspended, or already was — idempotent), `false` if no such id. The `lifecycle/suspend`
    /// executor drives this via the loop's apply-step (defer-to-loop, like terminate).
    pub fn suspend(&mut self, id: &SessionId) -> bool {
        match self.sessions.get_mut(id) {
            Some(s) => {
                s.suspend();
                true
            }
            None => false,
        }
    }

    /// RESUME a registered session by id (§lifecycle I4): clear its `suspended` bit so the loop schedules it
    /// again (held inbound replays). Returns `true` if the session exists, `false` if absent. Idempotent.
    pub fn resume(&mut self, id: &SessionId) -> bool {
        match self.sessions.get_mut(id) {
            Some(s) => {
                s.resume();
                true
            }
            None => false,
        }
    }

    /// Is the session `id` suspended (host-scheduler bit)? `false` for an absent id (nothing to hold). The
    /// loop consults each session's [`HostedSession::is_suspended`] directly; this is the by-id convenience.
    pub fn is_suspended(&self, id: &SessionId) -> bool {
        self.sessions.get(id).is_some_and(|s| s.is_suspended())
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
        let Some(s) = self.sessions.get_mut(id) else {
            // Addressed to no live session — a misrouted/late event. Count it distinctly from a delivered
            // turn, then report "unknown" up (None) as before.
            self.metrics.record_delivery_to_unknown_session();
            tracing::warn!(
                target: "cdz_agent_host::session",
                session_id = id.as_str(),
                "delivery to unknown session (routed nowhere)"
            );
            return None;
        };
        let started = std::time::Instant::now();
        let outcome = s.deliver(body, cause).await;
        self.metrics
            .record_turn_latency_us(crate::metrics::micros_u64(started.elapsed()));
        self.metrics.record_turn(outcome.is_ok());
        // Trace the turn outcome at the same boundary the metric records. An errored turn logs the kernel
        // reason at warn (a supervisor signal); a successful turn at debug (routine, filtered out at info).
        match &outcome {
            Ok(()) => tracing::debug!(
                target: "cdz_agent_host::session",
                session_id = id.as_str(),
                "turn ok"
            ),
            Err(e) => tracing::warn!(
                target: "cdz_agent_host::session",
                session_id = id.as_str(),
                error = ?e,
                "turn errored"
            ),
        }
        // §4c v0.3 merge-back: after a SUCCESSFUL turn, fold this session's new name-store writes into the
        // canonical shared store so the next-spawned (or next-reconciled) session sees them. Idempotent — a
        // turn that wrote nothing re-merges as a no-op (the session's log is already a prefix of what it was
        // handed). Gated on `outcome.is_ok()`: a turn that ERRED (KernelError — session/log corruption or an
        // invalid transition) may have left partial/invalid store state, and folding that into the SHARED
        // store would leak it to every future session — so an errored turn's writes are NOT published.
        // Only when the host is canonical-backed AND the session has a store attached.
        if outcome.is_ok() {
            if let Some(canonical) = &mut self.canonical {
                if let Some(session_store) = s.session().name_store() {
                    canonical.merge_appends_from(session_store);
                }
            }
        }
        Some(outcome)
    }

    /// Read-only access to a hosted session (for a status query / inspection). `None` = unknown id.
    pub fn get(&self, id: &SessionId) -> Option<&HostedSession> {
        self.sessions.get(id)
    }

    /// Find the registry key ([`SessionId`]) of the session whose GENESIS HASH equals `genesis` — a
    /// content-addressed lookup that does NOT assume the id IS `hex(genesis)`. A [`SessionId`] is opaque
    /// host-assigned metadata: a spawned child is registered under its genesis-hash hex, but a root / named
    /// session can be registered under a VANITY id (e.g. `"concierge"`) — the same SessionId-is-opaque
    /// distinction as the §I5 bounce arc. So any code holding a `Hash` (e.g. `Session::parent()`) that needs
    /// the corresponding registry entry must resolve it by matching `genesis_hash()`, not by hex-ing the hash
    /// into a `SessionId`. `None` = no registered session has that genesis hash (gone / never present).
    ///
    /// O(sessions) scan — fine at v0 scale (the only caller is the per-death I7 emit). A genesis-hash→id
    /// reverse index is the O(1) path if this ever moves onto a hot loop (same revisit trigger as I5's
    /// group-scan cost note).
    ///
    /// DETERMINISTIC on a (contract-impossible) duplicate: a genesis_hash is UNIQUE across registered
    /// sessions BY CONTRACT — the host mints a FRESH `spawn_nonce` (OS entropy) per spawn, and
    /// `genesis_hash = Hash::of(genesis Event over (reducer, spawn_nonce, parent))`, so two live sessions
    /// sharing one would require a 256-bit nonce/preimage collision (cryptographically unreachable, per
    /// v-agent-harness). The kernel does NOT enforce this (it never sees the live-session set — the nonce is
    /// caller-supplied), so this uses `min_by` on the `SessionId` rather than `find` (HashMap iteration order
    /// is unstable): a stable winner instead of an arbitrary one, should the contract ever be violated (or a
    /// degenerate test reuse a nonce). The `debug_assert` makes such a break surface LOUDLY in tests instead
    /// of silently mis-routing a `ChildExited` to an arbitrary parent (#2484 c-a). Scope: REGISTERED spawned
    /// sessions — a `fork_for_query` view deliberately reuses its parent's provenance (same genesis_hash) but
    /// is never registered, so it can't appear here.
    pub fn session_id_by_genesis_hash(&self, genesis: &Hash) -> Option<SessionId> {
        let mut matches = self
            .sessions
            .iter()
            .filter(|(_, s)| &s.genesis_hash() == genesis);
        let first = matches.next().map(|(id, _)| id.clone())?;
        // Fold in any further matches, keeping the lexicographically-smallest SessionId as the stable winner.
        // Under the fresh-nonce uniqueness contract there is exactly one match, so this is a no-op; the branch
        // exists only to make a contract violation deterministic + loud rather than iteration-order-dependent.
        let winner = matches.fold(first, |best, (id, _)| {
            debug_assert!(
                false,
                "genesis_hash uniqueness invariant violated: >1 registered session shares a genesis hash \
                 (expected unique by the fresh-spawn_nonce contract) — routing to the min SessionId"
            );
            if id < &best {
                id.clone()
            } else {
                best
            }
        });
        Some(winner)
    }

    /// FREEZE `controller`'s transitive spawn-descendant set into a concrete [`ResourcePredicate::OneOf`]
    /// (§lifecycle I6): the authority a session has to `lifecycle/*` (spawn/suspend/resume/terminate) a
    /// target is "target ∈ my transitive Spawned-descendants" — but the kernel's declarative
    /// [`ResourcePredicate::DescendantOf`] fails closed (it can't walk the registry at authorize-time,
    /// §4b replay-safety). So the HOST computes the descendant set HERE (it has the registry + each session's
    /// [`spawned_children`](HostedSession::spawned_children)) and bakes it into a `OneOf` grant the authorizer
    /// CAN evaluate. Re-compute + re-install after each new `Spawned` edge (a spawn changes the tree).
    ///
    /// Walks the durable spawn-edge tree from `controller` (traversal order is irrelevant — the returned SET
    /// doesn't depend on it; this uses a Vec+pop worklist, i.e. depth-first): each edge's `child_hash` IS the
    /// child's SessionId (genesis-hash-hex), so the descendant set is those hex ids. A child not currently
    /// registered (terminated + removed) contributes its own id but no further descendants (its subtree is
    /// gone with it). Cycle-safe (a `visited` set) though the spawn DAG is acyclic by construction (a child's
    /// id is provenance-derived from its parent, so it can't be its own ancestor). Returns `OneOf(∅)` (admits
    /// nothing) for a controller with no descendants — correctly denying lifecycle control of any peer.
    pub fn descendant_set_of(&self, controller: &SessionId) -> ResourcePredicate {
        let mut out: Vec<std::sync::Arc<str>> = Vec::new();
        // Alloc-light (#2447 review c2): key the visited set on SessionId (Eq+Hash, cheap Arc<str> clone) —
        // no per-id String; and push the SessionId's OWN `Arc<str>` (`id.0.clone()`) rather than copying its
        // bytes into a fresh Arc.
        let mut visited: std::collections::HashSet<SessionId> = std::collections::HashSet::new();
        // Seed the frontier with the controller's DIRECT children (the controller itself is not its own
        // descendant — a session can't lifecycle-control itself via this authority).
        let mut frontier: Vec<SessionId> = self
            .sessions
            .get(controller)
            .map(child_ids)
            .unwrap_or_default();
        while let Some(id) = frontier.pop() {
            if !visited.insert(id.clone()) {
                continue; // already recorded (cycle-guard / diamond)
            }
            // Descend into this child's own children (transitive), if it's still registered.
            if let Some(child) = self.sessions.get(&id) {
                frontier.extend(child_ids(child));
            }
            out.push(id.0.clone()); // reuse SessionId's internal Arc<str>
        }
        ResourcePredicate::OneOf(out)
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
        let removed = self.sessions.remove(id);
        if removed.is_some() {
            self.metrics.record_session_removed();
        }
        removed
    }

    /// TERMINATE a registered session by id (§lifecycle I5): install the durable `Terminated` marker on its
    /// log (via [`HostedSession::terminate`] → [`Session::terminate`]) AND remove it from the registry, so it
    /// no longer schedules or accepts deliveries. Returns:
    /// - `Some(Ok(hash))` — terminated; `hash` is the marker event hash (for cause-link / audit). The session
    ///   is dropped from the registry (its final terminated state is discarded here; a caller that needs the
    ///   tombstone should snapshot it first — a durable-store retention pass is a later slice).
    /// - `Some(Err(FoldRefused))` — the session was ALREADY terminated (idempotent-by-rejection); it is left
    ///   as-is (NOT removed a second time — it was already handled by the first terminate).
    /// - `None` — no such session id (already gone / never registered) — a no-op, matching [`remove`](Self::remove).
    ///
    /// `by` is the terminating controller's identity (its genesis hash = its SessionId), recorded in the
    /// marker. This is the registry-mutation half of the `lifecycle/terminate` effect; the in-flight-`Emit`
    /// BOUNCE (a terminated/absent target → `delivery-failure` to the sender) is enforced at the loop routing
    /// arm, not here. Marking the log BEFORE removing means a concurrent query between the two still sees a
    /// terminated (not merely absent) session; once removed, an `Emit` to it bounces via registry-absence.
    pub async fn terminate(
        &mut self,
        id: &SessionId,
        by: Hash,
        reason: String,
    ) -> Option<Result<Hash, KernelError>> {
        let session = self.sessions.get_mut(id)?;
        // §lifecycle I7 — snapshot the child's identity + its parent link BEFORE `terminate` (which moves
        // `reason`) and BEFORE `remove` (which drops the session from the registry). The child's genesis
        // hash IS its SessionId; `parent()` is the spawning session's genesis hash (None for a root).
        let child_hash = session.genesis_hash();
        let parent = session.session().parent();
        let i7_reason = reason.clone();
        let outcome = session.terminate(by, reason).await;
        // Only drop it from the registry on a FRESH termination — an already-terminated session
        // (FoldRefused) was already removed by its first terminate, so there is nothing to remove and the
        // rejection is surfaced as-is.
        if outcome.is_ok() {
            // §session-directory I5 death-retract (concierge-ruled OPTION B, scan-on-death): a terminated
            // session is AUTO-EVICTED from every group it was a member of (multicast stops fanning out to
            // it). The dead session's id IS its genesis hash = the member value in a session-group OR-set, so
            // retract by that hash. Done BEFORE `remove` while the id is in hand.
            self.retract_dead_member_from_groups(id);
            self.remove(id);
            // §lifecycle I7 host-mechanism half: after the durable Terminated marker + the I5 group-evict,
            // emit a `ChildExited` INBOUND into the PARENT's inbox (host-as-sender, reusing the deliver/inbox
            // path — NOT an authz-gated `lifecycle/*` EFFECT the parent performs; the family string is just
            // the Inbound content-type the prelude I7 supervisor matches on before `decode_child_exited`).
            // A terminate is a FAILURE close, so the outcome is `CloseOutcome::Failure(reason)`. Fire only
            // when the child HAS a parent that is STILL registered — a root session (`None`) or a gone/
            // already-terminated parent = no-op, no bounce (the supervisor, if any, is gone too).
            //
            // ⚠ Resolve the parent by its GENESIS HASH, not by `hex(parent_hash)` as a SessionId: the id is
            // OPAQUE host metadata (a spawned child gets genesis-hash-hex, but a top-level NAMED supervisor
            // can be registered under a vanity id like "concierge"). Hex-ing the hash into a SessionId would
            // miss a vanity-id parent → the signal would be SILENTLY DROPPED and the supervisor would never
            // learn its child died. `session_id_by_genesis_hash` matches on `genesis_hash()` (PR #2481 c1,
            // same SessionId-is-opaque root cause as the §I5 bounce arc).
            if let Some(parent_hash) = parent {
                if let Some(parent_id) = self.session_id_by_genesis_hash(&parent_hash) {
                    let payload = cdz_kernel::event_ast::encode_child_exited(
                        &child_hash,
                        &cdz_kernel::event::CloseOutcome::Failure(i7_reason),
                    );
                    let body = EventBody::Inbound {
                        content_type: cdz_kernel::event::ContentType {
                            family: "lifecycle/child-exited".into(),
                            version: 1,
                        },
                        payload: cdz_kernel::effect::Payload::Inline(payload.into()),
                    };
                    // `cause = None` (v1) — a provenance link to the terminate is a cheap later add. The
                    // parent's turn outcome is not surfaced here: a supervisor fold that errors is the
                    // parent's own concern, logged at the deliver boundary.
                    let _ = self.deliver(&parent_id, body, None).await;
                }
            }
        }
        Some(outcome)
    }

    /// §session-directory I5 — evict a dead member from every group in the host-owned CANONICAL name store
    /// (concierge-ruled OPTION B, SCAN-ON-DEATH). For each group where `dead`'s SessionId (= its genesis
    /// hash) has a live add-tag, append an observed-`remove` carrying that exact tag (add-wins: retracts
    /// precisely the add it observed, so a concurrent re-add with a fresh tag would survive — though a dead
    /// session won't re-add). No-op when there is no canonical store (the v0.2 per-session-store mode has no
    /// host-writable groups — the host can only READ a session's own store via the kernel, so death-retract
    /// there would need a kernel mut-seam / owner-driven path; v0 ships the canonical-store path the shared
    /// directory uses). `suspend` is transparent (only terminate evicts).
    ///
    /// ⚠ COST (concierge-flagged, documented): O(groups × ops) per death — it scans every group's OR-set log
    /// in the canonical store. Fine at v0 scale (deaths rare, few groups); the REVISIT TRIGGER is a central
    /// group-store OR a measured perf issue (whichever first), at which point a session→groups reverse index
    /// (or the central store's own index) becomes the O(1) path. See the directory-i5 index note.
    fn retract_dead_member_from_groups(&mut self, dead: &SessionId) {
        let Some(canonical) = &mut self.canonical else {
            return; // per-session-store mode: no host-writable group set to retract from
        };
        // The member value is the dead session's genesis hash (= its SessionId hex parsed back to a Hash). A
        // non-hex id can't be a group member value → nothing to retract.
        let Some(dead_hash) = Hash::from_hex(dead.as_str()) else {
            return;
        };
        // Collect (group, tag) pairs for the dead member's LIVE adds (an add-tag not already covered by a
        // remove), grouped by name — snapshot first (can't borrow the store's logs while appending).
        let mut retract: Vec<(String, (Hash, u64))> = Vec::new();
        {
            use std::collections::HashSet;
            let ops = canonical.to_group_ops(); // Vec<(name, MemberOp)> over all groups
                                                // Per group, the set of removed tags (to skip already-retracted adds).
            let removed: HashSet<(&str, (Hash, u64))> = ops
                .iter()
                .filter(|(_, op)| !op.add)
                .map(|(n, op)| (n.as_str(), op.tag))
                .collect();
            for (name, op) in &ops {
                if op.add && op.member == dead_hash && !removed.contains(&(name.as_str(), op.tag)) {
                    retract.push((name.clone(), op.tag));
                }
            }
        }
        if retract.is_empty() {
            return;
        }
        for (name, (origin, seq)) in &retract {
            // add_op appends the op directly to the canonical store's group log (host-owned; not via the
            // kernel effect path — the host is the authority for a death-retract, not a guest reducer).
            let _ = canonical.add_op(
                name,
                cdz_kernel::name_store::MemberOp::remove(dead_hash, *origin, *seq),
            );
        }
        tracing::info!(
            dead = %dead.as_str(),
            groups = retract.len(),
            "session-directory I5: auto-evicted a terminated session from its groups (scan-on-death)"
        );
    }

    /// How many sessions are registered.
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Is the registry empty (no running sessions)?
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// The host's live metric surface — registry counters bumped internally at spawn/remove/deliver.
    pub fn metrics(&self) -> &crate::metrics::HostMetrics {
        &self.metrics
    }

    /// The s2n-quic-dc-metrics registry the host records into — the daemon hands this to the export backend
    /// (which drains it on its reporting interval) and registers the executor set's `EffectMetrics` into it,
    /// so all the daemon's metrics share one registry the exporter reports over.
    pub fn registry(&self) -> &crate::metrics::Registry {
        &self.registry
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

    /// A stand-in genesis reducer (mirrors v-harness-bootstrap's reducer_genesis.cdz contract): folds each
    /// well-known genesis-setup family's payload into the contracted KV key, requesting no effects.
    struct GenesisRecordingReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for GenesisRecordingReducer {
        async fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
            if let EventBody::Inbound {
                content_type,
                payload: Payload::Inline(bytes),
            } = &event.body
            {
                let key = match content_type.family.as_ref() {
                    genesis_ct::ROOT => Some(genesis_ct::KV_ROOT_IDENTITY.to_vec()),
                    genesis_ct::AUTHORIZER => Some(genesis_ct::KV_AUTHORIZER_HASH.to_vec()),
                    genesis_ct::CONTEXT => Some(genesis_ct::KV_CONTEXT.to_vec()),
                    _ => None,
                };
                if let Some(k) = key {
                    kv.put(k, bytes.to_vec());
                }
            }
            FoldOutput::none()
        }
    }

    #[tokio::test]
    async fn seed_genesis_folds_the_setup_events_into_kv() {
        // seed_genesis delivers genesis/root|authorizer|context as ordinary inbound events; the genesis
        // reducer folds each into its contracted KV key. Deny-all authz is fine — the setup events request no
        // effects. Proves the host side of the bootstrap contract with v-harness-bootstrap.
        let mut session = HostedSession::genesis(
            Hash::of(b"genesis-reducer-v1"),
            Box::new(GenesisRecordingReducer),
            Box::new(Authorizer::deny_all()),
            CompositeExecutor::new(),
        );

        session
            .seed_genesis(
                b"root-identity-bytes",
                Some(b"authz-hash-bytes"),
                Some(b"ctx"),
            )
            .await
            .expect("genesis seed folds without error");

        let kv = session.session().kv();
        assert_eq!(
            kv.get(b"bootstrap/root-identity"),
            Some(&b"root-identity-bytes"[..])
        );
        assert_eq!(
            kv.get(b"bootstrap/authorizer-hash"),
            Some(&b"authz-hash-bytes"[..])
        );
        assert_eq!(kv.get(b"bootstrap/context"), Some(&b"ctx"[..]));
    }

    #[tokio::test]
    async fn seed_genesis_root_only_leaves_optional_keys_absent() {
        // A minimal boot seeds only the root; the optional authorizer/context keys stay absent.
        let mut session = HostedSession::genesis(
            Hash::of(b"genesis-reducer-v1"),
            Box::new(GenesisRecordingReducer),
            Box::new(Authorizer::deny_all()),
            CompositeExecutor::new(),
        );
        session
            .seed_genesis(b"just-root", None, None)
            .await
            .expect("root-only seed ok");
        let kv = session.session().kv();
        assert_eq!(kv.get(b"bootstrap/root-identity"), Some(&b"just-root"[..]));
        assert_eq!(kv.get(b"bootstrap/authorizer-hash"), None);
        assert_eq!(kv.get(b"bootstrap/context"), None);
    }

    #[test]
    fn genesis_ct_contract_constants_are_pinned_to_their_wire_literals() {
        // A DRIFT TRIPWIRE for the guest↔host bootstrap contract (contract with v-harness-bootstrap's
        // reducer_genesis.cdz). The behavioral seed_genesis tests above route through GenesisRecordingReducer,
        // which matches these families SYMBOLICALLY (genesis_ct::ROOT => genesis_ct::KV_ROOT_IDENTITY), so a
        // rename of the literal (e.g. "genesis/root" → "genesis/boot") moves BOTH sides together and those
        // tests STILL PASS — yet the real v-harness-bootstrap reducer hard-codes the literal family strings +
        // KV keys, so it would silently stop recognizing the event and genesis would break with nothing in the
        // gate to catch it. Pin the literal bytes here so any change to a wire value is a loud test failure
        // that forces a coordinated bump on BOTH sides. If you MUST change a literal, bump VERSION and
        // coordinate with v-harness-bootstrap first.
        assert_eq!(genesis_ct::ROOT, "genesis/root");
        assert_eq!(genesis_ct::AUTHORIZER, "genesis/authorizer");
        assert_eq!(genesis_ct::CONTEXT, "genesis/context");
        assert_eq!(genesis_ct::VERSION, 1);
        assert_eq!(genesis_ct::KV_ROOT_IDENTITY, b"bootstrap/root-identity");
        assert_eq!(genesis_ct::KV_AUTHORIZER_HASH, b"bootstrap/authorizer-hash");
        assert_eq!(genesis_ct::KV_CONTEXT, b"bootstrap/context");
    }

    #[test]
    fn hosted_session_genesis_hash_delegates_and_is_stable() {
        // HostedSession::genesis_hash() is the host's SessionId primitive (operator ruling: SessionId =
        // genesis-hash-hex). Pin that it delegates to the underlying Session + is stable across calls.
        let s = HostedSession::genesis(
            Hash::of(b"reducer-A"),
            Box::new(GenesisRecordingReducer),
            Box::new(Authorizer::deny_all()),
            CompositeExecutor::new(),
        );
        assert_eq!(
            s.genesis_hash(),
            s.session().genesis_hash(),
            "delegates to Session"
        );
        assert_eq!(s.genesis_hash(), s.genesis_hash(), "stable across calls");
    }

    #[test]
    fn two_same_reducer_sessions_get_distinct_genesis_hashes_uniqueness_gap_closed() {
        // §lifecycle I2a CLOSED the SessionId-uniqueness gap this test used to PIN. Previously
        // `Session::genesis(reducer)` carried no entropy, so two sessions over the SAME reducer produced
        // IDENTICAL genesis events → IDENTICAL genesis_hash → a SessionId COLLISION (a second same-reducer
        // session would clobber the first in the registry). Now `HostedSession::genesis` mints a fresh
        // OS-random spawn nonce into the seq-0 Genesis event (via `mint_spawn_nonce`), so two same-reducer
        // sessions get DIFFERENT nonces → DIFFERENT genesis_hash → distinct SessionIds. This asserts the
        // gap is closed (assert_ne, flipped from the old assert_eq pin).
        let a = HostedSession::genesis(
            Hash::of(b"same-reducer"),
            Box::new(GenesisRecordingReducer),
            Box::new(Authorizer::deny_all()),
            CompositeExecutor::new(),
        );
        let b = HostedSession::genesis(
            Hash::of(b"same-reducer"),
            Box::new(GenesisRecordingReducer),
            Box::new(Authorizer::deny_all()),
            CompositeExecutor::new(),
        );
        assert_ne!(
            a.genesis_hash(),
            b.genesis_hash(),
            "two same-reducer sessions now get DISTINCT genesis_hashes (per-session spawn nonce, \
             §lifecycle I2a) — the SessionId=genesis-hash uniqueness gap is closed"
        );
    }

    /// Env-gate for a REAL-reducer E2E: require BOTH a reducer-component-path env (`reducer_env`) AND
    /// `CDZ_STORE` (the handle-lowered reducer's value-heap runtime + transitive nfc resolve from it).
    /// Returns `Some((reducer_path, store_dir))` when both are set; `None` (clean SKIP, printed) when NEITHER
    /// is — a bare `cargo test` with no nix env stays green. A HALF-wired env (exactly one set) PANICS: a
    /// broken CI setup must fail loud, never masquerade as a clean skip. Single-sources this skip/fail-loud
    /// contract across the real-reducer E2Es so they can't drift (#2315 review).
    ///
    /// A present-but-EMPTY var (`CDZ_STORE=""`) counts as UNSET, not set: `.ok()` alone maps `Some("")`
    /// through, which would drive `ComponentStore::open("")` (CWD-relative) and silently mask a misconfigured
    /// CI env — so filter empties out and treat them as absent (#2320 review).
    fn require_reducer_and_store_or_skip(
        test_name: &str,
        reducer_env: &str,
    ) -> Option<(String, String)> {
        let non_empty = |var: &str| std::env::var(var).ok().filter(|v| !v.is_empty());
        let reducer_path = non_empty(reducer_env);
        let store_dir = non_empty("CDZ_STORE");
        match (reducer_path, store_dir) {
            (None, None) => {
                eprintln!("SKIP {test_name}: {reducer_env} + CDZ_STORE unset (or empty)");
                None
            }
            (Some(_), None) => panic!(
                "{test_name}: {reducer_env} is set but CDZ_STORE is not — the handle-lowered reducer's \
                 runtime dep needs the component store to resolve its transitive cadenza:nfc/normalize (§23)"
            ),
            (None, Some(_)) => {
                panic!("{test_name}: CDZ_STORE is set but {reducer_env} is not — nothing to drive")
            }
            (Some(r), Some(s)) => Some((r, s)),
        }
    }

    /// END-TO-END: drive the REAL rcdzc-compiled genesis reducer (v-harness-bootstrap's
    /// `reducer_genesis.cdz`) through the HOST's async path — `HostedSession::seed_genesis` → the async
    /// `AsyncComponentReducer` fold (§23 dep-compose, landed #2256) → session KV — and assert every seeded
    /// setup payload lands under its contracted `genesis_ct` KV key. Where `seed_genesis_folds_the_setup_events_into_kv`
    /// uses a Rust MOCK reducer (so a literal rename moves both sides + still passes, see
    /// `genesis_ct_contract_constants_are_pinned_to_their_wire_literals`), THIS proves the host drives the
    /// ACTUAL Cadenza reducer that hard-codes the literal families/keys — the true host↔reducer contract.
    ///
    /// Env-gated skip-on-unset (mirrors the kernel's `reducer_cadenza_b1_e2e`): a bare `cargo test` with no
    /// nix env stays green; CI (v-nix, per the #2249 b1 pattern) sets both. Two vars are REQUIRED together —
    /// - `GENESIS_REDUCER_COMPONENT` — the compiled `reducer-cadenza-genesis` component bytes.
    /// - `CDZ_STORE` — the hash-keyed `<sha256hex>.wasm` component store; the genesis reducer imports the
    ///   value-heap runtime (it lowers compounds to handles), whose OWN transitive `cadenza:nfc/normalize`
    ///   the §23 compose resolves by name from this store. Set one without the other and we FAIL LOUD (a
    ///   half-wired CI env must not masquerade as a clean skip).
    #[tokio::test]
    async fn real_genesis_reducer_folds_setup_events_through_the_host_async_path() {
        use cdz_kernel::wasm_host::AsyncComponentReducer;

        let Some((reducer_path, store_dir)) = require_reducer_and_store_or_skip(
            "real_genesis_reducer_folds_setup_events_through_the_host_async_path",
            "GENESIS_REDUCER_COMPONENT",
        ) else {
            return;
        };

        let bytes = std::fs::read(&reducer_path).unwrap_or_else(|e| {
            panic!("GENESIS_REDUCER_COMPONENT={reducer_path:?} set but unreadable: {e}")
        });
        let reducer = AsyncComponentReducer::from_component_bytes(&bytes)
            .unwrap_or_else(|e| panic!("reducer_genesis must be a valid component: {e:?}"));

        // The genesis reducer lowers compounds to value-heap handles, so it declares a `cadenza:runtime/heap`
        // dep — resolve every declared dep's bytes from the store, then attach the store so the §23 compose
        // can also resolve the runtime's OWN transitive `cadenza:nfc/normalize` by name. Resolve via
        // `ComponentStore::get_by_hash` (NOT a manual `<store>/<hash>.wasm` read): it's the production store
        // reader the fold itself uses, so the test exercises the same content-address SHA-256 verify (#2210)
        // — a corrupted/substituted blob is a host-side error (ContentAddressMismatch → a Compose error in
        // production), not composed silently (#2261 review).
        let store = cdz_kernel::component_store::ComponentStore::open(&store_dir);
        let deps = reducer.deps().to_vec();
        assert!(
            !deps.is_empty(),
            "the real genesis reducer must declare a cadenza:runtime/heap dep (it folds via the value heap)"
        );
        let mut resolved = Vec::with_capacity(deps.len());
        for dep in &deps {
            let dep_bytes = store.get_by_hash(&dep.hash).unwrap_or_else(|e| {
                panic!(
                    "CDZ_STORE={store_dir:?} could not resolve genesis reducer dep {:?} (hash {}): {e:?}",
                    dep.import_name,
                    dep.hash.to_hex()
                )
            });
            resolved.push((dep.clone(), dep_bytes));
        }
        let reducer = reducer
            .with_resolved_deps(resolved)
            .with_component_store(store);

        // Drive the HOST path: a genesis session over the REAL reducer, then seed the three setup events.
        // Deny-all authz is fine — genesis setup events request no effects.
        let mut session = HostedSession::genesis(
            Hash::of(b"real-genesis-reducer"),
            Box::new(reducer),
            Box::new(Authorizer::deny_all()),
            CompositeExecutor::new(),
        );
        session
            .seed_genesis(
                b"root-identity-bytes",
                Some(b"authz-hash-bytes"),
                Some(b"ctx-blob"),
            )
            .await
            .expect("the real genesis reducer folds the seed events through the host async path");

        // Each setup payload landed under its contracted genesis_ct KV key — the real reducer's hard-coded
        // family→key routing agrees with the host's genesis_ct literals, end to end.
        let kv = session.session().kv();
        assert_eq!(
            kv.get(genesis_ct::KV_ROOT_IDENTITY),
            Some(&b"root-identity-bytes"[..]),
            "genesis/root payload folds to bootstrap/root-identity"
        );
        assert_eq!(
            kv.get(genesis_ct::KV_AUTHORIZER_HASH),
            Some(&b"authz-hash-bytes"[..]),
            "genesis/authorizer payload folds to bootstrap/authorizer-hash"
        );
        assert_eq!(
            kv.get(genesis_ct::KV_CONTEXT),
            Some(&b"ctx-blob"[..]),
            "genesis/context payload folds to bootstrap/context"
        );
    }

    #[tokio::test]
    async fn with_sink_persists_a_sessions_events_durably() {
        // with_sink attaches a durable LogStore: the events a session appends during a turn are persisted,
        // so recovering the log file replays them. Proves the durable-log attach seam (the daemon uses this
        // per-session when [log].backend = file).
        use cdz_kernel::log_store::LogStore;
        // A UNIQUE, proven-fresh per-run temp dir (crate::testutil) so concurrent runners (cargo's parallel
        // harness + the nix test-check) never share the log file (#1988/#1991/#1995 review family).
        let dir = crate::testutil::unique_temp_dir("with-sink");
        let path = dir.join("session-durable.log");

        let sink = LogStore::open(&path).expect("open log store");
        let mut host = AgentHost::new();
        let id = SessionId::new("durable");
        host.spawn(id.clone(), now_host().with_sink(Box::new(sink)));
        // Drive a turn — the session appends events (Inbound + the Now dispatch/result), each written
        // through to the sink. Assert the turn actually SUCCEEDED (Some(Ok)) — a KernelError turn would
        // still append the Inbound, so a log-length-only check could pass on a failed turn (#1988 review).
        assert!(
            matches!(host.deliver(&id, inbound_go(), None).await, Some(Ok(()))),
            "the durable session ran its turn without a kernel error"
        );

        // The in-memory log has the full event stream (Genesis + the turn's events)…
        let in_mem = host.get(&id).unwrap().session().log().len();
        assert!(in_mem > 1, "the session appended a turn's events");
        // …and recovering the durable file replays every event appended AFTER the sink was attached. The
        // sink is attached post-genesis (with_sink is a builder over genesis), so the Genesis event predates
        // it and isn't persisted through this sink — the durable log holds the (in_mem - 1) later events.
        let recovered = LogStore::recover(&path).expect("recover the durable log");
        assert_eq!(
            recovered.events.len(),
            in_mem - 1,
            "every event appended after the sink was attached was persisted (all but pre-sink Genesis)"
        );
        assert!(
            !recovered.events.is_empty(),
            "the turn's events reached durable storage"
        );
        // Clean up the unique per-run dir (best-effort — the process is ending; a leftover unique dir can't
        // poison another run since the pid+seq is distinct each time).
        let _ = std::fs::remove_dir_all(&dir);
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
    async fn host_metrics_record_at_the_lifecycle_and_turn_boundaries() {
        // The metric surface records at the host boundaries: spawn (install), deliver (turn ok/err +
        // unknown-session), remove — into the registry. Registry Counters are drain-on-report with no value
        // getter, so drive a real sequence (must not panic) + assert the registry reports over the recorded
        // metrics (the export path). The per-boundary increment logic is exercised; the values reach the
        // exporter, not a test getter.
        let mut host = AgentHost::new();
        host.spawn(SessionId::new("a"), now_host());
        host.spawn(SessionId::new("b"), now_host());
        // A delivered turn to a known session (the `now` reducer completes Ok).
        host.deliver(&SessionId::new("a"), inbound_go(), None).await;
        // A delivery to an UNKNOWN id — recorded distinctly (deliveries_to_unknown_session), not as a turn.
        host.deliver(&SessionId::new("ghost"), inbound_go(), None)
            .await;
        // Remove one.
        host.remove(&SessionId::new("b"));

        assert!(
            host.registry().try_take_current_metrics_line().is_some(),
            "the registry reports over the recorded host metrics"
        );
    }

    #[tokio::test]
    async fn respawn_records_the_replaced_session_as_removed() {
        // A spawn onto an existing id drops the old session without a remove() call; spawn() records that
        // implicit drop (record_session_removed) so the installed/removed counters stay balanced on restarts.
        // Drive it (no panic) — the increment is on the replace path in spawn(); values reach the exporter.
        let mut host = AgentHost::new();
        let id = SessionId::new("worker");
        host.spawn(id.clone(), now_host());
        host.spawn(id.clone(), now_host()); // restart — replaces + records a removal
        assert_eq!(host.len(), 1, "replace, not add");
        assert!(host.registry().try_take_current_metrics_line().is_some());
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
    async fn terminate_marks_the_log_then_removes_the_session_from_the_registry() {
        // §lifecycle I5 slice-1: AgentHost::terminate installs the durable Terminated marker on the
        // session's log AND drops it from the registry. Returns the marker hash. A subsequent Emit to it
        // bounces via registry-absence (enforced at the loop arm, a later slice) — here we pin the
        // registry-mutation half: marked terminated, then gone.
        let mut host = AgentHost::new();
        let id = SessionId::new("victim");
        host.spawn(id.clone(), now_host());
        assert!(host.contains(&id));

        let by = Hash::of(b"controller-session");
        let out = host.terminate(&id, by, "operator kill".into()).await;
        match out {
            Some(Ok(marker)) => {
                // A real event hash was returned (not the zero/default) — the marker was appended.
                assert_ne!(
                    marker,
                    Hash::of(b""),
                    "terminate returns the marker event hash"
                );
            }
            other => panic!("expected Some(Ok(marker)) on a fresh terminate, got {other:?}"),
        }
        // The session is gone from the registry — no longer scheduled or deliverable.
        assert!(
            !host.contains(&id),
            "a terminated session is removed from the registry"
        );
        assert_eq!(host.len(), 0);
    }

    #[tokio::test]
    async fn terminate_evicts_the_dead_session_from_its_groups_i5_scan_on_death() {
        // §session-directory I5 (scan-on-death, option B): terminating a session AUTO-EVICTS it from every
        // group it was a member of in the host-owned canonical store, while OTHER members stay. Suspend would
        // be transparent; only terminate evicts.
        use cdz_kernel::name_store::{MemberOp, NameStore};
        const GROUP: &str = "session/room/lobby";

        // Two members: the victim (whose session we'll terminate) + a survivor. A group member value is a
        // session's genesis hash; the victim's SessionId must be that hash's hex so terminate can retract it.
        let victim_hash = Hash::of(b"victim-genesis");
        let survivor_hash = Hash::of(b"survivor-genesis");
        let origin = Hash::of(b"adder-origin");

        // Canonical store with both members added to the group (each add tagged (origin, seq) — the OR-set).
        let mut canonical = NameStore::new();
        canonical
            .add_op(GROUP, MemberOp::add(victim_hash, origin, 0))
            .unwrap();
        canonical
            .add_op(GROUP, MemberOp::add(survivor_hash, origin, 1))
            .unwrap();

        let mut host = AgentHost::with_canonical_store(canonical);
        // Register a terminatable session under the victim's genesis-hash-hex id.
        let victim_id = SessionId::new(victim_hash.to_hex());
        host.spawn(victim_id.clone(), now_host());

        // Both members present before the death.
        assert_eq!(
            host.canonical_store().unwrap().resolve_all(GROUP).unwrap(),
            [victim_hash, survivor_hash].into_iter().collect(),
            "both members are in the group before termination"
        );

        // Terminate the victim → I5 scan-on-death retracts it from the group.
        host.terminate(&victim_id, Hash::of(b"ctl"), "kill".into())
            .await
            .expect("victim present")
            .expect("fresh terminate");

        // The victim is evicted; the survivor remains (observed-remove is precise).
        let members = host.canonical_store().unwrap().resolve_all(GROUP).unwrap();
        assert!(
            !members.contains(&victim_hash),
            "the terminated session is auto-evicted from the group (I5)"
        );
        assert!(
            members.contains(&survivor_hash),
            "a non-terminated member stays in the group"
        );
    }

    #[tokio::test]
    async fn terminate_evicts_the_dead_session_from_ALL_its_groups_i5_multi_group() {
        // §session-directory I5 edge (the all-groups loop in retract_dead_member_from_groups): a session that
        // belongs to MULTIPLE groups is evicted from EVERY one on death, while each group's other member
        // survives. The single-group test above doesn't exercise the loop over `to_group_ops`' distinct names
        // nor the per-group observed-remove. Here the victim is in 3 groups (one shared with a survivor, one
        // solo, one with a different survivor) → all 3 retracts fire, each precise.
        use cdz_kernel::name_store::{MemberOp, NameStore};
        const LOBBY: &str = "session/room/lobby";
        const OPS: &str = "session/room/ops";
        const SOLO: &str = "session/room/solo";

        let victim_hash = Hash::of(b"multi-victim-genesis");
        let survivor_a = Hash::of(b"survivor-a-genesis");
        let survivor_b = Hash::of(b"survivor-b-genesis");
        let origin = Hash::of(b"adder-origin");

        // Victim in all three; a different co-member in lobby + ops; solo has only the victim.
        let mut canonical = NameStore::new();
        canonical
            .add_op(LOBBY, MemberOp::add(victim_hash, origin, 0))
            .unwrap();
        canonical
            .add_op(LOBBY, MemberOp::add(survivor_a, origin, 1))
            .unwrap();
        canonical
            .add_op(OPS, MemberOp::add(victim_hash, origin, 2))
            .unwrap();
        canonical
            .add_op(OPS, MemberOp::add(survivor_b, origin, 3))
            .unwrap();
        canonical
            .add_op(SOLO, MemberOp::add(victim_hash, origin, 4))
            .unwrap();

        let mut host = AgentHost::with_canonical_store(canonical);
        let victim_id = SessionId::new(victim_hash.to_hex());
        host.spawn(victim_id.clone(), now_host());

        // Present in all three before the death.
        for g in [LOBBY, OPS, SOLO] {
            assert!(
                host.canonical_store()
                    .unwrap()
                    .resolve_all(g)
                    .unwrap()
                    .contains(&victim_hash),
                "victim is in {g} before termination"
            );
        }

        // One terminate → scan-on-death retracts the victim from EVERY group it was in.
        host.terminate(&victim_id, Hash::of(b"ctl"), "kill".into())
            .await
            .expect("victim present")
            .expect("fresh terminate");

        let store = host.canonical_store().unwrap();
        // Evicted everywhere.
        for g in [LOBBY, OPS, SOLO] {
            assert!(
                !store.resolve_all(g).unwrap().contains(&victim_hash),
                "victim auto-evicted from {g} (I5 all-groups)"
            );
        }
        // Each group's OTHER member is untouched (observed-remove is precise per group).
        assert!(
            store.resolve_all(LOBBY).unwrap().contains(&survivor_a),
            "lobby co-member survives"
        );
        assert!(
            store.resolve_all(OPS).unwrap().contains(&survivor_b),
            "ops co-member survives"
        );
        // The solo group is now empty (only the victim had been in it) — a clean empty resolve, not a panic.
        assert!(
            store.resolve_all(SOLO).unwrap().is_empty(),
            "solo group is empty after the sole member is evicted"
        );
    }

    /// §lifecycle I7 test reducer: a parent supervisor that folds a `lifecycle/child-exited` Inbound. It
    /// decodes the canonical codec payload and records the exited child's hash + the failure reason into KV,
    /// so a test can observe that the host actually delivered the supervision signal. Any other inbound is a
    /// no-op (a real supervisor would also decide restart/escalate — out of scope for the host-mechanism E2E).
    struct ChildExitedFoldingReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for ChildExitedFoldingReducer {
        async fn fold(&self, event: &Event, kv: &mut Kv) -> FoldOutput {
            if let EventBody::Inbound {
                content_type,
                payload,
            } = &event.body
            {
                if content_type.matches_family("lifecycle/child-exited") {
                    // Pin the v1 wire contract: a silent version bump/drop (family+payload unchanged) must
                    // NOT pass unnoticed (PR #2481 c2 — value-assert the contract, not just the family).
                    kv.put(
                        b"exit-version".to_vec(),
                        content_type.version.to_string().into_bytes(),
                    );
                    if let Payload::Inline(bytes) = payload {
                        if let Ok((child, outcome)) =
                            cdz_kernel::event_ast::decode_child_exited(bytes)
                        {
                            kv.put(b"exited-child".to_vec(), child.to_hex().into_bytes());
                            let reason = match outcome {
                                cdz_kernel::event::CloseOutcome::Failure(r) => r.into_bytes(),
                                cdz_kernel::event::CloseOutcome::Success(_) => b"success".to_vec(),
                            };
                            kv.put(b"exit-reason".to_vec(), reason);
                        }
                    }
                }
            }
            FoldOutput::none()
        }
    }

    #[tokio::test]
    async fn terminating_a_child_delivers_child_exited_into_the_parents_inbox_i7() {
        // §lifecycle I7 host-mechanism half: when a child is TERMINATED, the host emits a `ChildExited`
        // Inbound (family "lifecycle/child-exited", canonical codec payload) into the PARENT's inbox — the
        // input the prelude supervisor folds. End-to-end: spawn a supervisor parent, spawn a real child under
        // it, terminate the child, and observe the parent folded the signal (child hash + failure reason).
        let mut host = AgentHost::new();
        // Register the parent under a VANITY id (NOT its genesis-hash hex) — a top-level named supervisor is
        // exactly this case, and the id is OPAQUE host metadata. The emit path resolves the parent by
        // matching genesis_hash() (PR #2481 c1), NOT by hex-ing child.parent() into a SessionId — so this
        // test would FAIL against the old `SessionId::new(parent_hash.to_hex())` lookup (the signal would be
        // silently dropped), which is the regression it pins.
        let supervisor = HostedSession::genesis(
            Hash::of(b"supervisor-v1"),
            Box::new(ChildExitedFoldingReducer),
            Box::new(Authorizer::deny_all()),
            CompositeExecutor::new(),
        );
        let parent = SessionId::new("concierge"); // vanity id ≠ hex(genesis_hash)
        assert_ne!(
            parent.as_str(),
            supervisor.genesis_hash().to_hex(),
            "the parent is deliberately registered under a vanity id, not its genesis-hash hex"
        );
        host.spawn(parent.clone(), supervisor);

        // Spawn a real child UNDER the parent (records the parent→child edge; the child's SessionId is its
        // provenance-dependent genesis hash, and its `parent()` points back at the supervisor).
        let child_id = host
            .spawn_child(
                &parent,
                Hash::of(b"child-reducer-v1"),
                Box::new(GenesisRecordingReducer),
                Box::new(Authorizer::deny_all()),
                CompositeExecutor::new(),
            )
            .await
            .expect("parent present")
            .expect("child spawned");
        let child_hash = Hash::from_hex(child_id.as_str()).expect("child id is genesis-hash hex");

        // Nothing folded yet — the parent has seen no child-exited signal.
        assert!(
            host.get(&parent)
                .unwrap()
                .session()
                .kv()
                .get(b"exited-child")
                .is_none(),
            "no child-exited folded before the child dies"
        );

        // Terminate the child → host emits ChildExited into the supervisor's inbox.
        host.terminate(&child_id, Hash::of(b"ctl"), "boom".into())
            .await
            .expect("child present")
            .expect("fresh terminate");

        // The supervisor folded the signal: the exited child's hash + the Failure reason round-tripped
        // through the canonical codec.
        let kv = host.get(&parent).unwrap();
        let kv = kv.session().kv();
        assert_eq!(
            kv.get(b"exited-child").map(|v| v.to_vec()),
            Some(child_hash.to_hex().into_bytes()),
            "the parent folded ChildExited carrying the terminated child's hash"
        );
        assert_eq!(
            kv.get(b"exit-reason").map(|v| v.to_vec()),
            Some(b"boom".to_vec()),
            "a terminate is a Failure close — the reason round-trips to the supervisor"
        );
        // Pin the v1 wire contract: the emitted ContentType.version is 1 (PR #2481 c2).
        assert_eq!(
            kv.get(b"exit-version").map(|v| v.to_vec()),
            Some(b"1".to_vec()),
            "the ChildExited Inbound carries content_type.version == 1 (v1 wire contract)"
        );
        // The child itself is gone from the registry (terminate removed it).
        assert!(
            !host.contains(&child_id),
            "the terminated child is unregistered"
        );
    }

    #[test]
    fn session_id_by_genesis_hash_resolves_a_vanity_id_by_content_not_hex() {
        // #2484 c-a happy path: the resolver finds a session under an OPAQUE (vanity) id by matching its
        // genesis_hash, not by hex-ing the hash into a SessionId. This is the single-match (contract-normal)
        // case — exactly one session per genesis hash under the fresh-nonce contract.
        let mut host = AgentHost::new();
        let s = now_host();
        let genesis = s.genesis_hash();
        let vanity = SessionId::new("concierge");
        assert_ne!(
            vanity.as_str(),
            genesis.to_hex(),
            "registered under a vanity id, not its genesis-hash hex"
        );
        host.spawn(vanity.clone(), s);
        assert_eq!(
            host.session_id_by_genesis_hash(&genesis),
            Some(vanity),
            "resolves the vanity-id session by its genesis hash (content-addressed, not hex==id)"
        );
        // A hash no session carries → None (not a panic, not a false match).
        assert_eq!(
            host.session_id_by_genesis_hash(&Hash::of(b"no-such-session")),
            None,
            "an unknown genesis hash resolves to None"
        );
    }

    #[test]
    #[should_panic(expected = "genesis_hash uniqueness invariant violated")]
    fn session_id_by_genesis_hash_debug_asserts_on_a_duplicate_genesis_hash() {
        // #2484 c-a tripwire: the fresh-nonce contract makes a genesis-hash collision across registered
        // sessions cryptographically unreachable — the kernel does NOT enforce it (nonce is host-supplied), so
        // the resolver debug_asserts if the contract is ever violated (or a degenerate test reuses a nonce),
        // surfacing the break LOUDLY instead of silently mis-routing a ChildExited to an arbitrary parent. We
        // FORCE the degenerate case: two sessions built with the SAME (reducer_hash, spawn_nonce) → the SAME
        // genesis_hash, registered under two different vanity ids. In a debug build the resolver panics; in a
        // release build it would deterministically return the min SessionId (untested here — asserts compiled out).
        let mut host = AgentHost::new();
        let reducer = Hash::of(b"dup-reducer");
        let nonce = Hash::of(b"REUSED-nonce-contract-violation");
        let mk = || {
            HostedSession::genesis_with_nonce(
                reducer,
                nonce,
                Box::new(GenesisRecordingReducer),
                Box::new(Authorizer::deny_all()),
                CompositeExecutor::new(),
            )
        };
        let a = mk();
        let b = mk();
        assert_eq!(
            a.genesis_hash(),
            b.genesis_hash(),
            "same (reducer, nonce) ⇒ same genesis hash (the forced collision)"
        );
        let genesis = a.genesis_hash();
        host.spawn(SessionId::new("aaa"), a);
        host.spawn(SessionId::new("bbb"), b);
        // Two registered sessions share a genesis hash → the resolver trips the uniqueness debug_assert.
        let _ = host.session_id_by_genesis_hash(&genesis);
    }

    #[tokio::test]
    async fn terminating_a_root_session_emits_no_child_exited_no_bounce_i7() {
        // §lifecycle I7 edge: a ROOT session (parent() == None) has no supervisor to notify — terminating it
        // is a clean no-op on the emit path (no panic, no bounce). Proven by terminating a plain root session.
        let mut host = AgentHost::new();
        let root = SessionId::new("root");
        host.spawn(root.clone(), now_host());
        assert!(
            host.get(&root).unwrap().session().parent().is_none(),
            "a root session has no parent"
        );
        // Terminates cleanly (the emit arm sees parent()==None and does nothing).
        host.terminate(&root, Hash::of(b"ctl"), "kill".into())
            .await
            .expect("root present")
            .expect("fresh terminate");
        assert!(
            !host.contains(&root),
            "the root is unregistered after terminate"
        );
    }

    #[tokio::test]
    async fn terminating_a_child_whose_parent_is_gone_emits_nothing_i7() {
        // §lifecycle I7 edge: if the parent is no longer registered (already terminated / never present) when
        // the child dies, the emit is a no-op — no delivery-to-unknown, no panic. Terminate the parent FIRST,
        // then the child, and confirm the child terminate still completes cleanly.
        let mut host = AgentHost::new();
        let parent = SessionId::new("gone-parent");
        host.spawn(parent.clone(), now_host());
        let child_id = host
            .spawn_child(
                &parent,
                Hash::of(b"child-reducer-v1"),
                Box::new(GenesisRecordingReducer),
                Box::new(Authorizer::deny_all()),
                CompositeExecutor::new(),
            )
            .await
            .expect("parent present")
            .expect("child spawned");
        // Kill the parent first — now the child's parent() still points at it, but it's no longer registered.
        host.terminate(&parent, Hash::of(b"ctl"), "parent-dies".into())
            .await
            .expect("parent present")
            .expect("fresh terminate");
        assert!(!host.contains(&parent), "parent gone");
        // Terminating the child with a gone parent completes cleanly (emit no-op — parent not registered).
        host.terminate(&child_id, Hash::of(b"ctl"), "child-dies".into())
            .await
            .expect("child present")
            .expect("fresh terminate");
        assert!(!host.contains(&child_id), "child gone too");
    }

    #[tokio::test]
    async fn terminating_a_child_delivers_child_exited_even_when_the_parent_is_suspended_i7() {
        // §lifecycle I7 × I4 interaction (pins the intended semantics): the I7 emit calls AgentHost::deliver
        // on the parent DIRECTLY (host-as-sender inside terminate), which folds immediately — it does NOT go
        // through the loop's inbox channel, so the scheduler `suspended` bit (which only HOLDS inbound
        // arriving via the loop) does NOT hold it. So a SUSPENDED supervisor still receives + folds
        // ChildExited the moment its child dies. This is deliberate: a supervision signal is a durable fold
        // that must reach the supervisor's log regardless of scheduler state (a suspended supervisor resumes
        // to an already-recorded child death, not a lost one). If a future refactor routes the I7 emit through
        // the loop channel (and thus the suspend-hold), this test flips — forcing a conscious re-decision.
        let mut host = AgentHost::new();
        let supervisor = HostedSession::genesis(
            Hash::of(b"supervisor-v1"),
            Box::new(ChildExitedFoldingReducer),
            Box::new(Authorizer::deny_all()),
            CompositeExecutor::new(),
        );
        let parent = SessionId::new("concierge"); // vanity id (also exercises the c1 genesis-hash lookup)
        host.spawn(parent.clone(), supervisor);

        let child_id = host
            .spawn_child(
                &parent,
                Hash::of(b"child-reducer-v1"),
                Box::new(GenesisRecordingReducer),
                Box::new(Authorizer::deny_all()),
                CompositeExecutor::new(),
            )
            .await
            .expect("parent present")
            .expect("child spawned");
        let child_hash = Hash::from_hex(child_id.as_str()).expect("child id is genesis-hash hex");

        // SUSPEND the supervisor BEFORE the child dies — the scheduler bit is set.
        assert!(host.suspend(&parent), "supervisor suspended");
        assert!(host.is_suspended(&parent), "the suspend bit is set");

        // Terminate the child → the I7 emit delivers ChildExited to the (suspended) supervisor immediately.
        host.terminate(&child_id, Hash::of(b"ctl"), "boom".into())
            .await
            .expect("child present")
            .expect("fresh terminate");

        // The suspended supervisor folded it anyway (direct deliver bypasses the loop's suspend-hold), and it
        // is still suspended (the emit doesn't resume it — orthogonal scheduler state).
        let hosted = host.get(&parent).unwrap();
        assert!(
            hosted.is_suspended(),
            "the supervisor stays suspended (the I7 emit doesn't flip the scheduler bit)"
        );
        assert_eq!(
            hosted.session().kv().get(b"exited-child").map(|v| v.to_vec()),
            Some(child_hash.to_hex().into_bytes()),
            "a SUSPENDED supervisor still folds ChildExited (direct deliver bypasses the loop suspend-hold)"
        );
    }

    #[tokio::test]
    async fn terminating_an_absent_session_is_a_none_noop() {
        // No such id (already gone / never registered) → None, not a panic — matching remove().
        let mut host = AgentHost::new();
        let out = host
            .terminate(&SessionId::new("ghost"), Hash::of(b"ctl"), "x".into())
            .await;
        assert!(
            out.is_none(),
            "terminating an absent session is a None no-op"
        );
    }

    #[tokio::test]
    async fn hosted_session_terminate_marks_terminated_and_is_idempotent_by_rejection() {
        // The HostedSession seam directly (below the registry): terminate installs the tail marker →
        // is_terminated() flips true; a SECOND terminate on the already-terminated session returns
        // FoldRefused (idempotent-by-rejection, the kernel's contract) — never a second marker.
        let mut s = now_host();
        assert!(!s.is_terminated(), "live before terminate");
        let first = s.terminate(Hash::of(b"ctl"), "done".into()).await;
        assert!(first.is_ok(), "first terminate installs the marker");
        assert!(s.is_terminated(), "the Terminated tail marks it terminated");
        let second = s.terminate(Hash::of(b"ctl"), "again".into()).await;
        assert!(
            matches!(second, Err(KernelError::FoldRefused)),
            "a 2nd terminate on an already-terminated session is FoldRefused, got {second:?}"
        );
    }

    #[tokio::test]
    async fn spawn_child_registers_the_child_under_its_genesis_hash_and_records_the_parent_edge() {
        // §lifecycle I3: AgentHost::spawn_child builds a child with parent-provenance, registers it under
        // SessionId = hex(child genesis_hash), and records the durable parent→child edge on the parent's log.
        let mut host = AgentHost::new();
        let parent = SessionId::new("parent");
        host.spawn(parent.clone(), now_host());

        let out = host
            .spawn_child(
                &parent,
                Hash::of(b"child-reducer-v1"),
                Box::new(GenesisRecordingReducer),
                Box::new(Authorizer::deny_all()),
                CompositeExecutor::new(),
            )
            .await;
        let child_id = match out {
            Some(Ok(id)) => id,
            other => panic!("expected Some(Ok(child_id)), got {other:?}"),
        };
        // The child is registered, and its id IS its genesis-hash hex (the operator's SessionId ruling).
        assert!(host.contains(&child_id), "child registered under its id");
        assert_eq!(
            child_id.as_str(),
            host.get(&child_id).unwrap().genesis_hash().to_hex(),
            "child SessionId = hex(its genesis_hash)"
        );
        // The parent's log carries exactly one Spawned edge, whose child_hash is the child's genesis hash.
        let edges = host.get(&parent).unwrap().spawned_children();
        assert_eq!(edges.len(), 1, "parent recorded one spawn edge");
        assert_eq!(
            edges[0].to_hex(),
            child_id.as_str(),
            "the edge's child_hash is the child's genesis hash (= its id)"
        );
    }

    #[tokio::test]
    async fn spawn_child_with_nonce_registers_the_id_derive_genesis_hash_pre_computes() {
        // §lifecycle I3 spawn-executor LOAD-BEARING invariant: the child id that spawn_child_with_nonce
        // REGISTERS must equal what Session::derive_genesis_hash(reducer, nonce, Some(parent)) PRE-COMPUTES
        // from the same triple. This is what lets the lifecycle/spawn executor return the child SessionId
        // synchronously (via derive_genesis_hash) while the loop registers the child later (defer-to-loop)
        // with the SAME nonce — the two must match byte-for-byte or the returned id is a lie. If a refactor
        // ever re-minted the nonce loop-side, this test flips.
        let mut host = AgentHost::new();
        let parent = SessionId::new("parent");
        host.spawn(parent.clone(), now_host());
        let parent_genesis = host.get(&parent).unwrap().genesis_hash();

        let reducer_hash = Hash::of(b"child-reducer-v1");
        let spawn_nonce = Hash::of(b"executor-minted-nonce");
        // What the EXECUTOR would return synchronously (pre-computed, no Session built).
        let pre_computed =
            Session::derive_genesis_hash(reducer_hash, spawn_nonce, Some(parent_genesis));

        // What the LOOP registers (defer-to-loop), given the SAME nonce.
        let out = host
            .spawn_child_with_nonce(
                &parent,
                reducer_hash,
                spawn_nonce,
                Box::new(GenesisRecordingReducer),
                Box::new(Authorizer::deny_all()),
                CompositeExecutor::new(),
            )
            .await;
        let child_id = match out {
            Some(Ok(id)) => id,
            other => panic!("expected Some(Ok(child_id)), got {other:?}"),
        };
        assert_eq!(
            child_id.as_str(),
            pre_computed.to_hex(),
            "the registered child id == derive_genesis_hash's pre-computation (executor's returned id is real)"
        );
        assert!(host.contains(&child_id));
    }

    #[test]
    fn genesis_spawned_threads_parent_into_the_child_id_the_load_bearing_provenance_guarantee() {
        // The CENTRAL guarantee of genesis_spawned vs genesis (Copilot #2417): the child's genesis_hash (=
        // its SessionId) is PROVENANCE-dependent — it self-certifies its parent. Proven at the Session level
        // with a FIXED reducer + FIXED nonce so ONLY `parent` varies: a spawned child (parent=Some) and a
        // root (parent=None) with identical reducer+nonce get DIFFERENT genesis_hashes. Without this, a bug
        // that dropped parent (used None) would go unnoticed — the registry/edge tests above would still pass
        // (they never inspect the child's own genesis provenance). This pins that `parent` is threaded in.
        let reducer = Hash::of(b"same-reducer");
        let nonce = Hash::of(b"same-nonce");
        let parent = Hash::of(b"parent-genesis-hash");
        let as_root = Session::genesis_spawned(reducer, nonce, None).genesis_hash();
        let as_child = Session::genesis_spawned(reducer, nonce, Some(parent)).genesis_hash();
        assert_ne!(
            as_root, as_child,
            "same reducer+nonce but different parent ⇒ different genesis_hash — parent IS threaded into \
             the child id (the provenance guarantee genesis_spawned exists for)"
        );
        // And it is DETERMINISTIC in the parent: the same (reducer, nonce, parent) reproduces the same id
        // (replay-stable — the host derives the child SessionId from this).
        let as_child_again = Session::genesis_spawned(reducer, nonce, Some(parent)).genesis_hash();
        assert_eq!(
            as_child, as_child_again,
            "genesis_hash is deterministic in (reducer, nonce, parent) — replay-stable child id"
        );
    }

    #[tokio::test]
    async fn spawn_child_under_an_absent_parent_is_a_none_noop() {
        let mut host = AgentHost::new();
        let out = host
            .spawn_child(
                &SessionId::new("ghost-parent"),
                Hash::of(b"child-v1"),
                Box::new(GenesisRecordingReducer),
                Box::new(Authorizer::deny_all()),
                CompositeExecutor::new(),
            )
            .await;
        assert!(
            out.is_none(),
            "spawning under an absent parent is a None no-op"
        );
        assert_eq!(host.len(), 0, "no child registered");
    }

    #[tokio::test]
    async fn a_terminated_parent_cannot_spawn_a_child_edge_refused_no_orphan() {
        // A terminated parent's log refuses the record_spawn append (FoldRefused); spawn_child records the
        // edge BEFORE registering the child, so the whole spawn is refused with NO child left registered
        // (no orphan whose parent rejected the edge).
        let mut host = AgentHost::new();
        let parent = SessionId::new("dead-parent");
        // Terminate the parent HostedSession BEFORE registering it (installs the Terminated tail), then
        // spawn it into the registry so spawn_child finds a registered-but-terminated parent.
        let mut parent_session = now_host();
        parent_session
            .terminate(Hash::of(b"ctl"), "kill".into())
            .await
            .expect("parent terminates");
        host.spawn(parent.clone(), parent_session);
        let before = host.len();
        let out = host
            .spawn_child(
                &parent,
                Hash::of(b"child-v1"),
                Box::new(GenesisRecordingReducer),
                Box::new(Authorizer::deny_all()),
                CompositeExecutor::new(),
            )
            .await;
        assert!(
            matches!(out, Some(Err(KernelError::FoldRefused))),
            "spawning under a terminated parent is refused (FoldRefused), got {out:?}"
        );
        assert_eq!(
            host.len(),
            before,
            "no child registered — a terminated parent spawns no orphan"
        );
    }

    // Extract the sorted hex-id set from a ResourcePredicate::OneOf (for descendant-set assertions).
    fn oneof_set(p: &ResourcePredicate) -> Vec<String> {
        match p {
            ResourcePredicate::OneOf(v) => {
                let mut s: Vec<String> = v.iter().map(|a| a.as_ref().to_string()).collect();
                s.sort();
                s
            }
            other => panic!("expected OneOf, got {other:?}"),
        }
    }

    // Spawn a child of `parent` (deny-all child, no executors) and return its SessionId.
    async fn spawn_kid(host: &mut AgentHost, parent: &SessionId, reducer_tag: &[u8]) -> SessionId {
        match host
            .spawn_child(
                parent,
                Hash::of(reducer_tag),
                Box::new(GenesisRecordingReducer),
                Box::new(Authorizer::deny_all()),
                CompositeExecutor::new(),
            )
            .await
        {
            Some(Ok(id)) => id,
            other => panic!("spawn_child({reducer_tag:?}) failed: {other:?}"),
        }
    }

    #[tokio::test]
    async fn descendant_set_of_freezes_the_transitive_spawn_subtree() {
        // §lifecycle I6: descendant_set_of walks the spawn-edge tree TRANSITIVELY (children + grandchildren)
        // and freezes it into a OneOf(hex-id set). Build root → child → grandchild + a second child, assert
        // root's descendant set = {child, child2, grandchild} (all 3, transitively), NOT just direct children.
        let mut host = AgentHost::new();
        let root = SessionId::new("root");
        host.spawn(root.clone(), now_host());
        let child = spawn_kid(&mut host, &root, b"child-1").await;
        let child2 = spawn_kid(&mut host, &root, b"child-2").await;
        let grandchild = spawn_kid(&mut host, &child, b"grandchild-1").await;

        let mut want = vec![
            child.as_str().to_string(),
            child2.as_str().to_string(),
            grandchild.as_str().to_string(),
        ];
        want.sort();
        assert_eq!(
            oneof_set(&host.descendant_set_of(&root)),
            want,
            "root's frozen descendant set is its full transitive subtree (children + grandchild)"
        );
        // A leaf (grandchild) has no descendants → empty OneOf (admits nothing — can't lifecycle-control anyone).
        assert_eq!(
            oneof_set(&host.descendant_set_of(&grandchild)),
            Vec::<String>::new(),
            "a leaf session has an empty descendant set"
        );
        // The intermediate child's set = just its own child (the grandchild), NOT root or its sibling.
        assert_eq!(
            oneof_set(&host.descendant_set_of(&child)),
            vec![grandchild.as_str().to_string()],
            "an intermediate session's descendant set is only ITS subtree"
        );
    }

    #[test]
    fn descendant_set_of_an_absent_or_childless_controller_is_empty() {
        let mut host = AgentHost::new();
        host.spawn(SessionId::new("lonely"), now_host());
        // A registered session with no spawns → empty; admits no target (denies all lifecycle control).
        assert_eq!(
            oneof_set(&host.descendant_set_of(&SessionId::new("lonely"))),
            Vec::<String>::new()
        );
        // An absent controller → empty (no tree to walk), fail-closed.
        assert_eq!(
            oneof_set(&host.descendant_set_of(&SessionId::new("ghost"))),
            Vec::<String>::new()
        );
    }

    #[test]
    fn suspend_resume_flips_the_scheduler_bit_without_touching_the_log() {
        // §lifecycle I4 mechanism: suspend/resume flip the host-scheduler bit (NOT a log mutation) +
        // idempotent; a suspended session is NOT terminated (orthogonal). AgentHost by-id + HostedSession
        // direct both work; absent id = false.
        let mut host = AgentHost::new();
        let id = SessionId::new("worker");
        host.spawn(id.clone(), now_host());
        assert!(!host.is_suspended(&id), "starts schedulable");

        assert!(
            host.suspend(&id),
            "suspend a registered session returns true"
        );
        assert!(host.is_suspended(&id), "now suspended");
        // A suspended session is NOT terminated (suspend is a scheduler bit, no durable marker).
        assert!(!host.get(&id).unwrap().is_terminated());
        // Idempotent: suspending again is still true, still suspended.
        assert!(host.suspend(&id));
        assert!(host.is_suspended(&id));

        assert!(host.resume(&id), "resume returns true");
        assert!(!host.is_suspended(&id), "resumed → schedulable again");
        assert!(host.resume(&id), "resume is idempotent");

        // Absent id: suspend/resume/is_suspended all report absence, no panic.
        assert!(!host.suspend(&SessionId::new("ghost")));
        assert!(!host.resume(&SessionId::new("ghost")));
        assert!(!host.is_suspended(&SessionId::new("ghost")));
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

    #[test]
    fn add_executor_extends_the_live_sessions_mechanism_surface() {
        // I6a mechanism axis: a session born with only Now cannot perform Http; add_executor(Http, …) makes
        // it able to, mid-session. handles_family is the mechanism dimension the capability manifest probes.
        let mut hosted = HostedSession::genesis(
            Hash::of(b"grow-mechanism-v1"),
            Box::new(ClockAgent),
            Box::new(Authorizer::deny_all()),
            CompositeExecutor::new().with_effect(effect_ct::NOW, Box::new(ClockExecutor::new())),
        );
        assert!(hosted.executor.handles_family(effect_ct::NOW));
        assert!(
            !hosted.executor.handles_family(effect_ct::HTTP),
            "born without an Http executor"
        );

        hosted.add_executor(
            effect_ct::HTTP,
            Box::new(crate::HttpExecutor::new(NeverHttp)),
        );

        assert!(
            hosted.executor.handles_family(effect_ct::HTTP),
            "add_executor flipped Http from Absent to present, mid-session"
        );
        assert!(
            hosted.executor.handles_family(effect_ct::NOW),
            "the pre-existing Now executor is retained (add, not replace-all)"
        );
    }

    #[tokio::test]
    async fn set_authorizer_swaps_the_live_sessions_policy_surface() {
        // I6a policy axis: swapping the authorizer flips which effects authorize with the SAME executor set.
        let mut hosted = HostedSession::genesis(
            Hash::of(b"swap-policy-v1"),
            Box::new(ClockAgent),
            Box::new(Authorizer::deny_all()),
            CompositeExecutor::new().with_effect(effect_ct::NOW, Box::new(ClockExecutor::new())),
        );
        let now_req = || {
            EffectRequest::new(
                EffectKind::Now,
                String::new(),
                None,
                Timeliness::Interactive,
            )
        };

        // Before: deny_all → the Now effect is refused even though a Now executor is present.
        assert!(
            hosted.authz.authorize(&now_req()).await.is_err(),
            "deny_all refuses Now"
        );

        hosted.set_authorizer(Box::new(Authorizer::new(vec![Capability {
            kind: EffectKind::Now,
            predicate: ResourcePredicate::Any,
        }])));

        assert!(
            hosted.authz.authorize(&now_req()).await.is_ok(),
            "set_authorizer swapped in a policy that permits Now, mid-session"
        );
    }

    /// A never-called Http transport — `add_executor` only needs a constructible executor to register the
    /// family; the test probes `handles_family` (mechanism), never performs a request.
    struct NeverHttp;
    #[async_trait::async_trait(?Send)]
    impl crate::HttpTransport for NeverHttp {
        async fn request(
            &self,
            _m: crate::HttpMethod,
            _u: &str,
            _h: &[(String, String)],
            _b: Option<&[u8]>,
            _k: Hash,
        ) -> Result<crate::HttpResponse, String> {
            unreachable!("mechanism-surface test never performs the request")
        }
    }

    /// Project the capabilities manifest bytes for a given served-executor set + authorizer — the exact
    /// wire bytes the kernel folds, via its public API (mirrors the seed test's expectation helper).
    async fn expected_manifest(exec: &CompositeExecutor, authz: &Authorizer) -> Vec<u8> {
        let manifest = cdz_kernel::effect::project_manifest(
            effect_ct::ALL,
            |f| exec.handles_family(f),
            authz,
            effect_ct::probe_target,
        )
        .await;
        cdz_kernel::event_ast::encode_capability_manifest(&manifest)
    }

    #[tokio::test]
    async fn push_capabilities_changed_folds_the_new_manifest_after_a_mechanism_change() {
        // I6b: an agent SEEDED with a Now-only surface, then granted an Http executor mid-session, gets a
        // capabilities-changed push carrying the NEW manifest (Http now usable) — same wire shape as seed.
        // Authorizer permits both Now + Http broadly, so the manifest entry for Http tracks the MECHANISM
        // (executor present) flipping Absent→Granted when we add the executor.
        let authz = || {
            Authorizer::new(vec![
                Capability {
                    kind: EffectKind::Now,
                    predicate: ResourcePredicate::Any,
                },
                Capability {
                    kind: EffectKind::Http,
                    predicate: ResourcePredicate::Any,
                },
            ])
        };
        let mut hosted = HostedSession::genesis(
            Hash::of(b"cap-push-v1"),
            Box::new(CapabilityAwareAgent),
            Box::new(authz()),
            CompositeExecutor::new().with_effect(effect_ct::NOW, Box::new(ClockExecutor::new())),
        );

        // Baseline: seed the born-knowing manifest (Now only served → Http reads Absent).
        hosted.seed_capabilities().await;
        let now_only =
            CompositeExecutor::new().with_effect(effect_ct::NOW, Box::new(ClockExecutor::new()));
        assert_eq!(
            hosted.session().kv().get(b"capabilities"),
            Some(&expected_manifest(&now_only, &authz()).await[..]),
            "seeded baseline reflects the Now-only surface"
        );

        // MECHANISM change: register an Http executor mid-session, then push.
        hosted.add_executor(
            effect_ct::HTTP,
            Box::new(crate::HttpExecutor::new(NeverHttp)),
        );
        let surfaced = hosted.push_capabilities_changed().await;
        assert!(
            surfaced.is_empty(),
            "the push is answered inline, not surfaced"
        );

        // The reducer recorded the NEW manifest — Now+Http served, both Granted by the broad policy.
        let now_and_http = CompositeExecutor::new()
            .with_effect(effect_ct::NOW, Box::new(ClockExecutor::new()))
            .with_effect(
                effect_ct::HTTP,
                Box::new(crate::HttpExecutor::new(NeverHttp)),
            );
        assert_eq!(
            hosted.session().kv().get(b"capabilities"),
            Some(&expected_manifest(&now_and_http, &authz()).await[..]),
            "push_capabilities_changed folded the NEW manifest (Http now usable) after add_executor"
        );
    }

    #[tokio::test]
    async fn push_capabilities_changed_is_a_no_op_when_nothing_changed() {
        // I6b coalescing gate: pushing with NO capability change since the last-known manifest folds
        // nothing — a session whose surface didn't move gets no spurious capabilities-changed.
        let mut hosted = HostedSession::genesis(
            Hash::of(b"cap-noop-v1"),
            Box::new(CapabilityAwareAgent),
            Box::new(Authorizer::deny_all()),
            CompositeExecutor::new().with_effect(effect_ct::NOW, Box::new(ClockExecutor::new())),
        );
        hosted.seed_capabilities().await; // establishes the baseline
        let after_seed = hosted
            .session()
            .kv()
            .get(b"capabilities")
            .map(|b| b.to_vec());
        let log_len_before = hosted.session().log().len();

        // No mutation between seed and push → the manifest hasn't moved → push is a no-op.
        let surfaced = hosted.push_capabilities_changed().await;
        assert!(surfaced.is_empty());
        assert_eq!(
            hosted
                .session()
                .kv()
                .get(b"capabilities")
                .map(|b| b.to_vec()),
            after_seed,
            "no capability change → the recorded manifest is untouched"
        );
        assert_eq!(
            hosted.session().log().len(),
            log_len_before,
            "no capability change → nothing appended to the log (the coalescing/gate)"
        );
    }
}
