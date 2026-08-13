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

/// A session's identity in the host registry — the session's genesis [`Hash`] (operator ruling: "session IDs
/// must be HASHES, not arbitrary strings"). The registry keys by this 32-byte `Copy` hash, NOT an owned
/// string. A human NAME (e.g. `"concierge"`, `"builder-42"`) is a SEPARATE display-only label carried
/// alongside the session ([`HostedSession::name`]), never the identity — an admin install supplies a name
/// and the host mints/derives the id as the session's genesis hash.
///
/// A `Hash` is a `Copy` `[u8; 32]` (cheaply clonable — the operator's binary-everywhere / no-owned-String
/// rule): keying + `spawn` + `session_ids()` copy 32 bytes with no allocation, and identity is uniform with
/// every other host handle (conn-id, reply-token). The hex form is for the WIRE (admin DTO) + TRACING only,
/// via [`SessionId::to_hex`] — never on the routing/storage path.
//
// The host drives sessions through the kernel's ASYNC loop (`Session::deliver`) so a long fold can
// cooperatively yield and sessions interleave (§15b). A reducer is therefore held as a `Box<dyn
// Reducer>` — the SINGLE reducer trait (operator "one async trait only"): a pure-Rust reducer writes
// a native `impl Reducer` (its `fold` runs to completion with no await point), and a wasm
// reducer uses `AsyncComponentReducer`. Both box directly as `Box<dyn Reducer>` — no wrapper.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct SessionId(pub Hash);

impl SessionId {
    /// The session id from its genesis [`Hash`] — the host's canonical identity (a spawned child's id IS its
    /// genesis hash; a root/admin-installed session's id is the genesis hash the host mints on install).
    pub fn new(genesis: Hash) -> Self {
        SessionId(genesis)
    }
    /// The underlying genesis [`Hash`] (the registry key; content-addressed identity).
    pub fn hash(&self) -> Hash {
        self.0
    }
    /// The hex rendering — for the WIRE (admin DTO) + TRACING/display ONLY, never routing/storage.
    pub fn to_hex(&self) -> String {
        self.0.to_hex()
    }
    /// The base64url rendering — the encode-only display form for TRACING output (operator directive
    /// 2026-08-12: base64url, not hex, at the permitted display + FS/S3 sites). Never routing/storage;
    /// there is no decode counterpart (a session-id is raw hash bytes, never parsed from a string).
    pub fn to_base64url(&self) -> String {
        self.0.to_base64url()
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
        .map(|h| SessionId::new(*h))
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

    /// Wrap an ALREADY-RECOVERED [`Session`] (§lifecycle I4b boot-recovery) — the recovery counterpart to
    /// [`genesis`](Self::genesis). Unlike the `genesis*` constructors (which MINT a fresh seq-0 Genesis), this
    /// takes the `Session` that [`Session::recover_from`](cdz_kernel::kernel::Session::recover_from) rebuilt by
    /// folding a durable log — so the recovered session keeps its own genesis-hash / SessionId / KV / open
    /// obligations (recovery reads the nonce back from the log, NEVER re-mints — the id is stable across a
    /// restart). The daemon's boot-recovery loop reads each durably-logged session
    /// (e.g. [`DynamoLogSink::read_recovered`](crate::dynamo_log::DynamoLogSink::read_recovered) /
    /// `LogStore::recover`) → `recover_from` → this → [`AgentHost::spawn`](crate::host::AgentHost::spawn) to
    /// re-register it, then re-drives the `RecoveryReport.open_effects` by idempotency key.
    ///
    /// The caller re-supplies the three per-session collaborators (reducer / authz / executor) exactly as for
    /// a fresh session — they are host-side wiring the log doesn't carry. `suspended` starts false: a
    /// recovered session is schedulable (a supervisor re-suspends if it wants — see the [`suspended`] doc,
    /// which is explicit that suspension does NOT survive recovery). A session recovered from a log whose tail
    /// is [`Terminated`](cdz_kernel::event::EventBody::Terminated) is detected by
    /// [`is_terminated`](Self::is_terminated) — the boot loop checks that and SKIPS re-registering it, so this
    /// constructor stays a pure wrap (it does not itself filter terminated sessions).
    ///
    /// [`suspended`]: HostedSession::suspended
    pub fn from_recovered(
        session: Session,
        reducer: Box<dyn Reducer>,
        authz: Box<dyn Authorize>,
        executor: CompositeExecutor,
    ) -> Self {
        HostedSession {
            session,
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
            .push_capabilities_changed(&mut *self.reducer, &*self.authz, &mut self.executor)
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
            .seed_capabilities(&mut *self.reducer, &*self.authz, &mut self.executor)
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
                &mut *self.reducer,
                &*self.authz,
                &mut self.executor,
            )
            .await
    }

    /// Like [`deliver`](Self::deliver) but SURFACES the `control/*` effects the reducer emitted this turn
    /// (via [`Session::deliver_control`]) so the host can answer host-surfaced control families — chiefly
    /// `control/signature` (the composable-component-calls signature-query, part-1). The common `deliver`
    /// path drops these; use this when the session may signature-query.
    ///
    /// Returns the surfaced [`ControlEffect`](cdz_kernel::effect::ControlEffect)s (usually empty). The caller
    /// (the loop) filters for [`effect_ct::SIGNATURE`], fetches each effect's target component bytes from the
    /// blob store (the target rides `ce.request.target` as a content-hash hex — the host holds the blob store,
    /// this session doesn't), and calls [`settle_signature_query`](Self::settle_signature_query) to reflect +
    /// fold the descriptor back. `control/summary` / `control/capabilities` in the returned set are handled by
    /// their own paths (fork-scrape / kernel-inline), not here.
    pub async fn deliver_surfacing_controls(
        &mut self,
        body: EventBody,
        cause: Option<Hash>,
    ) -> Result<Vec<cdz_kernel::effect::ControlEffect>, KernelError> {
        self.session
            .deliver_control(
                body,
                cause,
                &mut *self.reducer,
                &*self.authz,
                &mut self.executor,
            )
            .await
    }

    /// Answer ONE surfaced `control/signature` effect: reflect the target component's exported signature and
    /// FOLD the descriptor back into this session so the emitting reducer resumes with it (§signature-query
    /// part-1, the `ControlHostSurfaced` fold-back seam `Session::settle_effect_result` — the generalized
    /// successor to the original `settle_control_result`, folding by `EffectId`).
    ///
    /// `target_bytes` is the target component's bytes, which the CALLER fetched from the blob store by the
    /// effect's target hash (`ce.request.target` hex) — `HostedSession` is blob-store-free, so the loop does
    /// the async fetch + hands the bytes here. On `Some(bytes)`: reflect via the kernel's wasmtime-side
    /// [`component_signature_from_bytes_owned`](cdz_kernel::wasm_host::component_signature_from_bytes_owned)
    /// (the host has no wasmtime dep; this is the bytes-only kernel seam) and settle
    /// `EffectOutcome::Ok(Some(Inline(descriptor)))`. On `None` (target blob absent) OR a reflect failure
    /// (not a component / an undescribable type), settle a classified `EffectOutcome::err` so the reducer
    /// folds the error arm and RESUMES cleanly rather than hanging on an open effect. Returns whether the
    /// settle actually landed (`false` = the id was already settled / the session terminated — a benign
    /// no-op, per `settle_effect_result`).
    pub async fn settle_signature_query(
        &mut self,
        ce: &cdz_kernel::effect::ControlEffect,
        target_bytes: Option<&[u8]>,
    ) -> bool {
        use cdz_kernel::effect::Payload;
        use cdz_kernel::event::EffectOutcome;
        let outcome = match target_bytes {
            Some(bytes) => match cdz_kernel::wasm_host::component_signature_from_bytes_owned(bytes)
            {
                Ok(descriptor) => EffectOutcome::Ok(Some(Payload::Inline(descriptor.into()))),
                Err(e) => EffectOutcome::err(format!(
                    "control/signature: could not reflect the target component's signature: {e:?}"
                )),
            },
            None => EffectOutcome::err(
                "control/signature: the target component was not found in the blob store"
                    .to_string(),
            ),
        };
        self.session
            .settle_effect_result(
                ce.id,
                outcome,
                &mut *self.reducer,
                &*self.authz,
                &mut self.executor,
            )
            .await
    }

    /// Settle a DEFERRED effect on THIS session with a userspace-effect handler's reply outcome
    /// (userspace-effects I4, loop-side). A caller session performed a userspace effect that the I3
    /// [`UserspaceEffectExecutor`](crate::userspace_effect_exec::UserspaceEffectExecutor) FORWARDED to a
    /// handler + left OPEN (returned [`EffectOutcome::Deferred`]); the handler answered with an `effect/reply`
    /// that the [`ReplyExecutor`](crate::reply_exec::ReplyExecutor) validated into a
    /// [`ReplySettle`](crate::reply_exec::ReplySettle) `{caller, effect_id, outcome}`. The loop drains that
    /// command (see `apply_reply_settles`) and calls THIS on the `caller` session to fold `outcome` onto the
    /// open `effect_id`, resuming the caller's continuation — closing the request→forward→reply→settle loop.
    ///
    /// A thin wrapper over the same `Session::settle_effect_result` seam `settle_signature_query` uses (the
    /// family-agnostic deferred-settle, userspace-effects I2). Returns whether the settle landed (`false` = the
    /// id was already settled / the session terminated — a benign no-op, per `settle_effect_result`, so a
    /// stale/duplicate reply the token layer somehow let through still can't corrupt the log).
    pub async fn settle_reply(
        &mut self,
        effect_id: cdz_kernel::effect::EffectId,
        outcome: cdz_kernel::event::EffectOutcome,
    ) -> bool {
        self.session
            .settle_effect_result(
                effect_id,
                outcome,
                &mut *self.reducer,
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
            .fire_due_timers(now_ms, &mut *self.reducer, &*self.authz, &mut self.executor)
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
    /// WARNING: NOT a GLOBAL identity — a [`SessionId`] is OPAQUE host-assigned metadata: a spawned child gets
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
    /// can't be cloned out of this `HostedSession`, so the caller re-provides it as a `&mut dyn Reducer`),
    /// a MODEL-ONLY `authz` (a scoped capability so a summarize-fold can call the model but CANNOT take
    /// world-actions — SEC-F1), and an `executor` to serve that model call. Returns `Some(summary_bytes)`
    /// if the reducer emitted a `control/summary` effect with an inline payload, else `None` (it
    /// summarized elsewhere / didn't, emitted a blob payload, or the fork erred).
    pub async fn fork_for_query(
        &self,
        reducer: &mut dyn Reducer,
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
/// The host's shared handle to the canonical [`NameStore`](cdz_kernel::name_store::NameStore): an
/// `Rc<RefCell<..>>` so the host, and any on-loop executor that must resolve LIVE name registrations, hold the
/// SAME store (v-agent-harness ruling A, 2026-08-09). Single-threaded (`Rc`/`RefCell`, not `Arc`/`Mutex` — the
/// host loop is `!Send` by design, like the rest of the executor set). Sharing is replay-safe: the `NameStore`
/// is external mutable state, never rebuilt from the log, so its in-memory identity carries no determinism.
pub type SharedNameStore = std::rc::Rc<std::cell::RefCell<cdz_kernel::name_store::NameStore>>;

/// Constructed via [`new`](Self::new) / [`with_canonical_store`](Self::with_canonical_store) (each creates
/// the metrics registry). `Default` delegates to `new` (it can't be derived — the metrics registry is a
/// required collaborator, not a `Default` field — so the impl forwards to `new`).
pub struct AgentHost {
    sessions: HashMap<SessionId, HostedSession>,
    /// The §4c v0.3 canonical shared name store, if this host is share-backed (see
    /// [`AgentHost::with_canonical_store`]). `None` = share-less host. Held as a SHARED handle
    /// ([`SharedNameStore`] = `Rc<RefCell<NameStore>>`): the host owns it, each spawned session gets a
    /// replay-COPY at spawn + folds its appends back after each turn (the single-writer-per-name §4c model),
    /// AND on-loop executors that must resolve LIVE registrations (the userspace-effect fallback's
    /// [`HandlerResolver`](crate::userspace_effect_exec::HandlerResolver)) hold a CLONE of this `Rc` so a
    /// handler registered by a peer THIS turn is visible immediately — not one merge-back behind. Sharing is
    /// replay-safe: the `NameStore` is EXTERNAL mutable state (re-attached after recover, never rebuilt from
    /// the log — §4c / kernel `name_store` field doc), so determinism comes from the logged events, never the
    /// store's in-memory identity (v-agent-harness ruling A, 2026-08-09).
    canonical: Option<SharedNameStore>,
    /// The §4c canonical-store DURABILITY backend (AWS-backends arc I4a), if durability is enabled. `None` =
    /// no durability (the default — snapshotting is best-effort + opt-in, so existing tests/behavior are
    /// unchanged). When `Some` AND the host is canonical-backed, the host snapshots the canonical store after
    /// any [`deliver`](Self::deliver) turn that folded a session's name-store writes back (see the merge-back
    /// site in `deliver`), and [`with_canonical_store_restored`](Self::with_canonical_store_restored) restores
    /// from it on boot. A share-less OR snapshot-less host does nothing (zero overhead, unchanged behavior).
    name_snapshot: Option<Box<dyn crate::name_snapshot::NameStoreSnapshotStore>>,
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
            name_snapshot: None,
            registry,
            metrics,
        }
    }

    /// Enable the §4c v0.3 SHARED name store — a single host-owned canonical [`NameStore`](cdz_kernel::name_store::NameStore) that gives LIVE
    /// cross-session visibility of published pointers, replacing the per-hand-off export/replay bridge.
    /// Opt-in: a host built with [`new`](Self::new) stays share-less and every session keeps whatever store
    /// (or none) it was spawned with.
    ///
    /// Lifecycle (single-writer-per-name, conflict-free): the host holds ONE canonical store (a
    /// [`SharedNameStore`] `Rc<RefCell<..>>`); on [`spawn`](Self::spawn) a session gets a by-VALUE copy of it
    /// (a replay of `canonical.to_set_entries()`), so it's born seeing everyone's published pointers; after
    /// each [`deliver`](Self::deliver) turn the host folds that session's new writes back with
    /// `canonical.borrow_mut().merge_appends_from(session.name_store())`. The `Rc<RefCell<..>>` also lets an
    /// on-loop executor hold a clone to resolve LIVE registrations (ruling A) — the per-session copy is still
    /// the spawn/merge-back model, composing with the [`HostedSession::with_name_store`] seam.
    pub fn with_canonical_store(canonical: cdz_kernel::name_store::NameStore) -> Self {
        let registry = crate::metrics::Registry::new();
        let metrics = crate::metrics::HostMetrics::new(&registry);
        AgentHost {
            sessions: HashMap::new(),
            canonical: Some(std::rc::Rc::new(std::cell::RefCell::new(canonical))),
            name_snapshot: None,
            registry,
            metrics,
        }
    }

    /// Set the §4c canonical-store DURABILITY backend — a builder that makes the shared name directory
    /// SURVIVE a restart (AWS-backends arc I4a). After any [`deliver`](Self::deliver) turn that folds a
    /// session's name-store writes into the canonical store, the host `save`s the canonical
    /// [`snapshot_bytes`](cdz_kernel::name_store::NameStore::snapshot_bytes) through this store (best-effort:
    /// a failed save is LOGGED, never fails the turn — the in-memory canonical stays authoritative for the
    /// run). Pair with [`with_canonical_store`](Self::with_canonical_store) (or use
    /// [`with_canonical_store_restored`](Self::with_canonical_store_restored), which restores + sets this in
    /// one step) — a snapshot store on a SHARE-LESS host is inert (nothing mutates the canonical, so nothing
    /// is ever saved). Opt-in: a host built without this stays non-durable (unchanged behavior, zero overhead).
    pub fn with_name_snapshot_store(
        mut self,
        store: Box<dyn crate::name_snapshot::NameStoreSnapshotStore>,
    ) -> Self {
        self.name_snapshot = Some(store);
        self
    }

    /// Build a canonical-store-backed host whose canonical store is RESTORED from a durable snapshot backend
    /// (AWS-backends arc I4a restore-on-boot) — the boot-time dual of the mutation-hook. `load`s the latest
    /// snapshot from `store`; if `Some`, reconstructs the canonical [`NameStore`](cdz_kernel::name_store::NameStore)
    /// via [`from_snapshot_bytes`](cdz_kernel::name_store::NameStore::from_snapshot_bytes); if `None` (nothing
    /// saved yet) OR the snapshot is CORRUPT (a tampered/garbled blob — LOGGED), starts with an empty
    /// [`NameStore::new`]. The `store` is then RETAINED for ongoing saves (so the first mutation re-snapshots).
    ///
    /// A corrupt snapshot starts EMPTY rather than panicking (durability is best-effort — a bad blob must not
    /// wedge the daemon at boot); the warning names the failure so an operator can investigate. A `load` I/O
    /// error is likewise non-fatal (LOGGED, start empty) — the daemon boots and re-establishes state as
    /// sessions run, rather than refusing to start on a transient backend hiccup.
    pub async fn with_canonical_store_restored(
        store: Box<dyn crate::name_snapshot::NameStoreSnapshotStore>,
    ) -> Self {
        let restored = match store.load().await {
            Ok(Some(bytes)) => match cdz_kernel::name_store::NameStore::from_snapshot_bytes(&bytes)
            {
                Ok(store) => store,
                Err(e) => {
                    tracing::warn!(
                        target: "cdz_agent_host::name_snapshot",
                        error = ?e,
                        "canonical name-store snapshot is corrupt — starting with an empty store"
                    );
                    cdz_kernel::name_store::NameStore::new()
                }
            },
            Ok(None) => {
                // Fresh deployment — nothing saved yet. Start empty (the first mutation snapshots).
                cdz_kernel::name_store::NameStore::new()
            }
            Err(e) => {
                tracing::warn!(
                    target: "cdz_agent_host::name_snapshot",
                    error = %e,
                    "could not load the canonical name-store snapshot — starting with an empty store"
                );
                cdz_kernel::name_store::NameStore::new()
            }
        };
        Self::with_canonical_store(restored).with_name_snapshot_store(store)
    }

    /// The host-owned CANONICAL shared name store handle (`None` for a share-less host). The read-back dual of
    /// [`with_canonical_store`](Self::with_canonical_store): a driver observes the shared directory the host
    /// maintains (group memberships after death-retract, published pointers) via `.borrow()`, e.g.
    /// `host.canonical_store().unwrap().borrow().resolve_all(group)`. It ALSO hands an on-loop executor a
    /// CLONE of the `Rc` (a cheap refcount bump) so the userspace-effect fallback resolves handler
    /// registrations LIVE against the same store the host mutates (ruling A). The host owns the mutation policy
    /// (session-write fold-back + §I5 death-retract); a holder that only reads uses `.borrow()`.
    pub fn canonical_store(&self) -> Option<&SharedNameStore> {
        self.canonical.as_ref()
    }

    /// A CLONE of the canonical shared-store handle (`None` for a share-less host) — the seam an on-loop
    /// executor (the userspace-effect [`HandlerResolver`](crate::userspace_effect_exec::HandlerResolver))
    /// captures to resolve `effect/<family>` registrations LIVE at perform-time (ruling A). Cloning the `Rc`
    /// is an O(1) refcount bump; the executor `.borrow()`s it per resolve. Distinct from
    /// [`canonical_store`](Self::canonical_store) only in intent (an owned handle to keep vs a borrow to read).
    pub fn shared_canonical_store(&self) -> Option<SharedNameStore> {
        self.canonical.clone()
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
            Some(canonical) => session.with_name_store(replay_of(&canonical.borrow())),
            None => session,
        };
        let replaced = self.sessions.insert(id, session).is_some();
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
            session_id = id.to_base64url(),
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
        let child_id = SessionId::new(child_hash);

        // Record the durable parent→child edge FIRST: a terminated parent refuses the append (FoldRefused),
        // and we then register NOTHING — so a terminated session can never spawn a live orphan.
        let parent_session = self.sessions.get_mut(parent)?;
        if let Err(e) = parent_session.record_spawn(child_hash).await {
            return Some(Err(e));
        }
        // Edge recorded → register the child (reuses `spawn`: canonical-store replay + metrics + trace).
        self.spawn(child_id, child);
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
                session_id = id.to_base64url(),
                "delivery to unknown session (routed nowhere)"
            );
            return None;
        };
        let started = std::time::Instant::now();
        let outcome = s.deliver(body, cause).await;
        self.metrics
            .record_turn_latency_us(crate::metrics::micros_u64(started.elapsed()));
        self.record_turn_and_merge_back(id, &outcome).await;
        Some(outcome)
    }

    /// Like [`deliver`](Self::deliver) but ANSWERS host-surfaced `control/signature` effects the turn emitted
    /// (signature-query part-1): the reducer emits `control/signature` naming a target component, the host
    /// reflects the target's exported signature + folds the descriptor back so the reducer resumes with it.
    /// Delivers via [`HostedSession::deliver_surfacing_controls`], then for each surfaced `control/signature`
    /// resolves the target component's bytes through `factory` (by the effect's target hash —
    /// [`SessionFactory::fetch_blob`]) and calls [`HostedSession::settle_signature_query`] to reflect + settle
    /// (an absent/undescribable target settles a clean Err arm, so the reducer never hangs). `control/summary`
    /// / `control/capabilities` in the surfaced set are handled by their own paths, not here.
    ///
    /// The loop uses THIS for ordinary inbound (where the session may signature-query) and passes its
    /// `factory`; a caller with no factory (or a session that never signature-queries) can still use plain
    /// [`deliver`](Self::deliver). Same return shape + the same post-turn merge-back as `deliver`.
    pub async fn deliver_answering_signatures(
        &mut self,
        id: &SessionId,
        body: EventBody,
        cause: Option<Hash>,
        factory: Option<&mut (dyn crate::admin::SessionFactory + '_)>,
    ) -> Option<Result<(), KernelError>> {
        use cdz_kernel::effect::effect_ct;
        let Some(s) = self.sessions.get_mut(id) else {
            self.metrics.record_delivery_to_unknown_session();
            tracing::warn!(
                target: "cdz_agent_host::session",
                session_id = id.to_base64url(),
                "delivery to unknown session (routed nowhere)"
            );
            return None;
        };
        let started = std::time::Instant::now();
        // Surface the control effects this turn emitted (the plain `deliver` drops them).
        let deliver_result = s.deliver_surfacing_controls(body, cause).await;
        self.metrics
            .record_turn_latency_us(crate::metrics::micros_u64(started.elapsed()));
        let controls = match deliver_result {
            Ok(controls) => controls,
            Err(e) => {
                // A kernel error IS the turn outcome — record + merge-back (no writes on Err) + report it.
                let outcome = Err(e);
                self.record_turn_and_merge_back(id, &outcome).await;
                return Some(outcome);
            }
        };
        // Answer each surfaced control/signature: resolve the target component bytes via the factory's blob
        // store (the effect target is a content-hash hex), reflect + settle. A None factory / an absent target
        // settles the Err arm (the reducer resumes cleanly). Re-borrow the session per effect (settle needs
        // &mut) — the session is still registered (a signature query doesn't terminate it).
        let mut factory = factory;
        for ce in &controls {
            if !ce.request.content_type.matches_family(effect_ct::SIGNATURE) {
                continue;
            }
            // The target rides the effect as the raw content-hash bytes; resolve it to bytes through the
            // factory. The target is opaque Arc<[u8]>; a wrong-length target yields no hash → the None arm
            // (reconstruct via from_bytes, NOT from_hex — the hash is raw bytes, never parsed from a string).
            let target_bytes = match (
                factory.as_deref_mut(),
                <[u8; 32]>::try_from(ce.request.target.as_ref())
                    .ok()
                    .map(Hash::from_bytes),
            ) {
                (Some(f), Some(hash)) => f.fetch_blob(&hash).await.ok().flatten(),
                // No factory, or a non-UTF-8/non-hex target — no bytes to reflect (settle_signature_query
                // settles the Err arm, so the reducer resumes).
                _ => None,
            };
            if let Some(s) = self.sessions.get_mut(id) {
                s.settle_signature_query(ce, target_bytes.as_deref()).await;
            }
        }
        // Record the successful turn + run the merge-back (also folds any settle writes). ONE metric/trace tap.
        let outcome = Ok(());
        self.record_turn_and_merge_back(id, &outcome).await;
        Some(outcome)
    }

    /// The shared post-turn work both delivery paths run: record the turn metric + trace, then (on a
    /// SUCCESSFUL turn only) fold this session's new name-store writes into the canonical shared store +
    /// durably snapshot it. Factored out of [`deliver`] so [`deliver_answering_signatures`] runs the identical
    /// metric/merge-back/snapshot contract — ONE tap, one merge-back, no divergence.
    async fn record_turn_and_merge_back(
        &mut self,
        id: &SessionId,
        outcome: &Result<(), KernelError>,
    ) {
        self.metrics.record_turn(outcome.is_ok());
        // Trace the turn outcome at the same boundary the metric records (errored at warn — a supervisor
        // signal; ok at debug — routine).
        match outcome {
            Ok(()) => tracing::debug!(
                target: "cdz_agent_host::session",
                session_id = id.to_base64url(),
                "turn ok"
            ),
            Err(e) => tracing::warn!(
                target: "cdz_agent_host::session",
                session_id = id.to_base64url(),
                error = ?e,
                "turn errored"
            ),
        }
        // §4c v0.3 merge-back: fold the session's new name-store writes into the canonical shared store (only
        // on a successful turn — an errored turn may have left partial state that must NOT publish). Only when
        // the host is canonical-backed AND the session has a store. BEST-EFFORT snapshot: a save failure is
        // logged, never fails the turn.
        if outcome.is_ok() {
            if let Some(canonical) = &self.canonical {
                if let Some(session_store) =
                    self.sessions.get(id).and_then(|s| s.session().name_store())
                {
                    // Fold the session's new writes into the shared canonical, then (if durable) snapshot the
                    // result. The RefCell borrow is scoped to the merge + snapshot_bytes read and DROPPED
                    // before the async save — a RefCell borrow must not be held across an `.await` (and the
                    // borrow is over in-memory work only; the I/O save takes owned `bytes`).
                    let snapshot_bytes = {
                        let mut c = canonical.borrow_mut();
                        c.merge_appends_from(session_store);
                        self.name_snapshot.is_some().then(|| c.snapshot_bytes())
                    };
                    if let (Some(bytes), Some(snapshot_store)) =
                        (snapshot_bytes, &mut self.name_snapshot)
                    {
                        if let Err(e) = snapshot_store.save(&bytes).await {
                            tracing::warn!(
                                target: "cdz_agent_host::name_snapshot",
                                session_id = id.to_base64url(),
                                error = %e,
                                "failed to snapshot the canonical name store (durability best-effort; \
                                 the in-memory store is still authoritative for this run)"
                            );
                        }
                    }
                }
            }
        }
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
        let first = matches.next().map(|(id, _)| *id)?;
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
                *id
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
        // Alloc-light: key the visited set on SessionId (a Copy genesis `Hash` — Eq+Hash, no allocation per
        // id). The Cedar resource strings are rendered as each id's hex only when pushed onto `out` below
        // (the predicate is matched against the effect-target hex).
        let mut visited: std::collections::HashSet<SessionId> = std::collections::HashSet::new();
        // Seed the frontier with the controller's DIRECT children (the controller itself is not its own
        // descendant — a session can't lifecycle-control itself via this authority).
        let mut frontier: Vec<SessionId> = self
            .sessions
            .get(controller)
            .map(child_ids)
            .unwrap_or_default();
        while let Some(id) = frontier.pop() {
            if !visited.insert(id) {
                continue; // already recorded (cycle-guard / diamond)
            }
            // Descend into this child's own children (transitive), if it's still registered.
            if let Some(child) = self.sessions.get(&id) {
                frontier.extend(child_ids(child));
            }
            out.push(id.to_hex().into()); // Cedar resource = the id hex (descendant set is matched against effect-target hex)
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

    /// Settle a DEFERRED userspace effect on the `caller` session with a handler's reply outcome
    /// (userspace-effects I4, loop-side) — the registry-facing half of the `effect/reply` path, resolving the
    /// caller by id then delegating to [`HostedSession::settle_reply`]. The loop's `apply_reply_settles`
    /// drains a [`ReplySettle`](crate::reply_exec::ReplySettle) and calls this so the caller's OPEN effect
    /// resumes with the handler's answer. Returns whether the settle LANDED: `false` if the caller is ABSENT
    /// (gone/terminated between the forward and the reply — no session to settle, like [`terminate`](Self::terminate)'s
    /// `None`) OR the id was already settled / the session terminated (the idempotent no-op `settle_reply`
    /// reports). Benign either way — a late/stale reply can't corrupt a log.
    pub async fn settle_reply(
        &mut self,
        caller: &SessionId,
        effect_id: cdz_kernel::effect::EffectId,
        outcome: cdz_kernel::event::EffectOutcome,
    ) -> bool {
        match self.sessions.get_mut(caller) {
            Some(s) => s.settle_reply(effect_id, outcome).await,
            None => false,
        }
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
            // WARNING: Resolve the parent by its GENESIS HASH, not by `hex(parent_hash)` as a SessionId: the id is
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

    /// §6 supervision — REAP every self-CLOSED session and notify its parent. This is the normal-completion
    /// counterpart to the terminate-path §lifecycle I7 `child-exited` notify above: a reducer self-closes by
    /// returning [`FoldOutput::close`](cdz_kernel::reducer::FoldOutput::close), and the kernel appends the
    /// terminal [`EventBody::Closed`] INSIDE `deliver` + flips the resident [`Session::is_closed`] flag but
    /// does NOT touch the host registry (the host owns it) — so a closed session LINGERS registered until this
    /// reap drops it. The loop calls this once per iteration AFTER the deliver/lifecycle/reply-settle apply
    /// steps, so a session that self-closed during ANY deliver this turn (an external inbound, a resumed
    /// held-inbound replay, or a `child-completed` cascade) is caught in one place.
    ///
    /// For each closed session: snapshot its child hash + parent link + the reducer's verbatim
    /// [`CloseOutcome`](cdz_kernel::event::CloseOutcome) off the terminal `Closed` tip BEFORE mutating; evict
    /// it from every group (§I5 death-retract, same as terminate); remove it from the registry; then — if it
    /// had a parent STILL registered — deliver a `lifecycle/child-completed` INBOUND into the parent carrying
    /// [`encode_child_completed`](cdz_kernel::ast_marshal::encode_child_completed)`(child, outcome)`. A ROOT
    /// close (`parent == None`) is just reaped, no notify.
    ///
    /// Resolve the parent by GENESIS HASH via [`session_id_by_genesis_hash`](Self::session_id_by_genesis_hash),
    /// NEVER hex-as-`SessionId` — a vanity-id supervisor (e.g. `"concierge"`) would be missed and the signal
    /// silently dropped (same SessionId-is-opaque root cause as the §I7 arm + the §I5 bounce, PR#2481 c1).
    ///
    /// FIXPOINT: delivering `child-completed` may itself close the PARENT (a supervisor that completes when its
    /// last child does), so this loops until no registered session is closed — the whole close-cascade drains
    /// in one call. Bounded: each pass removes at least one session, and `deliver` never adds a closed one.
    pub async fn reap_closed_and_notify(&mut self) {
        loop {
            // Immutable snapshot pass — collect (id, child-hash, parent, outcome) for every closed session
            // BEFORE any mutation (the notify `deliver` below needs `&mut self`, and `remove` moves entries).
            let closed: Vec<(
                SessionId,
                Hash,
                Option<Hash>,
                cdz_kernel::event::CloseOutcome,
            )> = self
                .sessions
                .iter()
                .filter_map(|(id, s)| {
                    if !s.session().is_closed() {
                        return None;
                    }
                    // The terminal tip of a closed session IS `EventBody::Closed{outcome}` (the only path the
                    // flag flips); read the reducer's verbatim outcome off it. Skip defensively otherwise.
                    match &s.session().tip().body {
                        EventBody::Closed { outcome } => {
                            Some((*id, s.genesis_hash(), s.session().parent(), outcome.clone()))
                        }
                        _ => None,
                    }
                })
                .collect();
            if closed.is_empty() {
                return;
            }
            // The set of sessions closing in THIS batch (by genesis hash). A child-completed notify must NOT
            // be delivered to a parent that is itself in this set: the reap loop removes sessions one-by-one,
            // so a still-present but already-closed parent would otherwise receive the Inbound and — since the
            // kernel's `deliver` guards `is_terminated` but NOT `is_closed` — FOLD it PAST its terminal
            // `Closed` event, corrupting the durable-log terminal-tip invariant (Closed → Inbound) and making
            // that parent un-reapable on recovery (its tip is no longer `Closed`, so the outcome read misses
            // it). A closing parent is terminal and doesn't need its children's completions anyway; its OWN
            // completion propagates to the GRANDPARENT when this same pass processes it (the grandparent is not
            // in the closed set), so nothing is lost. (Order-independent: if the parent were processed first it
            // would already be `remove`d and the resolve below would be `None` — this set makes both orders
            // safe.) Reviewer c267d8431 finding.
            let closing_this_batch: std::collections::HashSet<Hash> = closed
                .iter()
                .map(|(_, child_hash, _, _)| *child_hash)
                .collect();
            for (id, child_hash, parent, outcome) in closed {
                // Finalize: evict from groups + drop from the registry (same as terminate's §I5 + `remove`).
                self.retract_dead_member_from_groups(&id);
                self.remove(&id);
                // Notify the parent (if any is still registered AND not itself closing this batch) via a
                // `lifecycle/child-completed` INBOUND.
                if let Some(parent_hash) = parent {
                    if !closing_this_batch.contains(&parent_hash) {
                        if let Some(parent_id) = self.session_id_by_genesis_hash(&parent_hash) {
                            let payload = cdz_kernel::ast_marshal::encode_child_completed(
                                &child_hash,
                                &outcome,
                            );
                            let body = EventBody::Inbound {
                                content_type: cdz_kernel::event::ContentType {
                                    family: "lifecycle/child-completed".into(),
                                    version: 1,
                                },
                                payload: cdz_kernel::effect::Payload::Inline(payload.into()),
                            };
                            // `cause = None` (v1) — mirrors the §I7 child-exited notify. A supervisor fold
                            // that errors on the notify is its own concern, logged at the deliver boundary.
                            let _ = self.deliver(&parent_id, body, None).await;
                        }
                    }
                }
            }
        }
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
    /// WARNING: COST (concierge-flagged, documented): O(groups × ops) per death — it scans every group's OR-set log
    /// in the canonical store. Fine at v0 scale (deaths rare, few groups); the REVISIT TRIGGER is a central
    /// group-store OR a measured perf issue (whichever first), at which point a session→groups reverse index
    /// (or the central store's own index) becomes the O(1) path. See the directory-i5 index note.
    fn retract_dead_member_from_groups(&mut self, dead: &SessionId) {
        let Some(canonical) = self.canonical.clone() else {
            return; // per-session-store mode: no host-writable group set to retract from
        };
        // One `borrow_mut` for the whole retract (read the group logs + append the remove ops) — no await
        // here, so a single scoped mutable borrow is correct + cheapest. The `Rc` clone above is an O(1)
        // refcount bump that sidesteps borrowing `self` twice (we mutate the shared store, not `self`).
        let mut canonical = canonical.borrow_mut();
        // The member value is the dead session's genesis hash (= its SessionId hex parsed back to a Hash). A
        // non-hex id can't be a group member value → nothing to retract.
        let Some(dead_hash) = Some(dead.hash()) else {
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
            dead = %dead.to_base64url(),
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
/// end-to-end shape of "an agent runs" (the crate's primary flow):
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
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
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

    #[tokio::test]
    async fn from_recovered_wraps_a_recovered_session_keeping_its_id() {
        use cdz_kernel::kernel::Session;
        use cdz_kernel::log_store::{Recovered, RecoveryKind};

        // A fresh session whose id (genesis hash) we capture, then simulate a restart: take its durable log,
        // recover_from it (the backend-agnostic recovery core), and wrap the recovered Session via
        // from_recovered — the I4b boot-recovery path. The wrapped session must keep the SAME genesis hash /
        // SessionId (recovery reads the nonce from the log, never re-mints) and start non-terminated +
        // schedulable.
        let original = now_host();
        let original_id = original.genesis_hash();
        // A never-delivered session's durable log is exactly its genesis event (log-decouple I5: read it off
        // the derived `genesis_ref`, not the resident Vec).
        let log = vec![original.session().genesis_ref().clone()];
        assert!(
            !log.is_empty(),
            "a genesis'd session has at least its seq-0 event"
        );

        let recovered = Recovered {
            events: log,
            kind: RecoveryKind::Clean,
            good_prefix_len: 0, // informational for this backend-agnostic path; recover_from ignores it
        };
        let (session, report) = Session::recover_from(recovered, &mut ClockAgent)
            .await
            .expect("a clean genesis log recovers");
        assert_eq!(report.kind, RecoveryKind::Clean);

        let executor =
            CompositeExecutor::new().with_effect(effect_ct::NOW, Box::new(ClockExecutor::new()));
        let hosted = HostedSession::from_recovered(
            session,
            Box::new(ClockAgent),
            Box::new(Authorizer::deny_all()),
            executor,
        );
        assert_eq!(
            hosted.genesis_hash(),
            original_id,
            "recovery keeps the same SessionId — the nonce is read from the log, never re-minted"
        );
        assert!(
            !hosted.is_terminated(),
            "a session recovered from a non-terminated log is not terminated"
        );
        assert!(
            !hosted.is_suspended(),
            "a recovered session starts schedulable (suspension does not survive recovery)"
        );
    }

    /// An agent that arms a timer for `deadline_ms` on inbound "go", and records "woke" in KV when the
    /// timer FIRES (a `TimerFired` event) — so a test can prove the host's timer sweep actually woke it.
    struct TimerAgent {
        deadline_ms: u64,
    }
    #[async_trait::async_trait(?Send)]
    impl Reducer for TimerAgent {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
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
        let id = host.spawn(SessionId::new(Hash::of(b"agent-1")), now_host());
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
        assert_eq!(
            hosted.session().kv().get(b"status").as_deref(),
            Some(&b"ran"[..])
        );
        assert_eq!(hosted.open_effects(), 0);
    }

    /// A stand-in genesis reducer (mirrors v-harness-bootstrap's reducer_genesis.cdz contract): folds each
    /// well-known genesis-setup family's payload into the contracted KV key, requesting no effects.
    struct GenesisRecordingReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for GenesisRecordingReducer {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
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
            kv.get(b"bootstrap/root-identity").as_deref(),
            Some(&b"root-identity-bytes"[..])
        );
        assert_eq!(
            kv.get(b"bootstrap/authorizer-hash").as_deref(),
            Some(&b"authz-hash-bytes"[..])
        );
        assert_eq!(kv.get(b"bootstrap/context").as_deref(), Some(&b"ctx"[..]));
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
        assert_eq!(
            kv.get(b"bootstrap/root-identity").as_deref(),
            Some(&b"just-root"[..])
        );
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
    ///
    /// A1 BYTES FOLD BOUNDARY (settled — no arming flag). Every real reducer fixture is A1-native on origin
    /// (`apply(list<u8>) -> list<u8>`, single canonical Event doc IN + value-form effect-list OUT), the
    /// `CDZ_KERNEL_BYTES_ABI` arming switch has been flipped + proven green in CI (all three reducer E2Es
    /// exercise non-vacuously), so the transition guard is GONE — collapsed, not carried as a dead migration
    /// flag (operator no-migration-layer directive; v-nix drops the export in lockstep). The Rust
    /// `AsyncComponentReducer::apply(kv, ct, payload, resumes)` signature is unchanged across A1 (it builds the
    /// Event doc internally via `ast_marshal::build_event_document`), and the genesis E2E drives via
    /// `seed_genesis`→`deliver`→`fold`→`apply`, so no apply-call reshape was ever needed. These E2Es now gate
    /// purely on their reducer-component env + `CDZ_STORE`, exactly like
    /// `real_pure_reducer_folds_an_event_through_the_a1_bytes_boundary`.
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
    /// - `CDZ_STORE` — the hash-keyed `<blake3hex>.wasm` component store; the genesis reducer imports the
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
            kv.get(genesis_ct::KV_ROOT_IDENTITY).as_deref(),
            Some(&b"root-identity-bytes"[..]),
            "genesis/root payload folds to bootstrap/root-identity"
        );
        assert_eq!(
            kv.get(genesis_ct::KV_AUTHORIZER_HASH).as_deref(),
            Some(&b"authz-hash-bytes"[..]),
            "genesis/authorizer payload folds to bootstrap/authorizer-hash"
        );
        assert_eq!(
            kv.get(genesis_ct::KV_CONTEXT).as_deref(),
            Some(&b"ctx-blob"[..]),
            "genesis/context payload folds to bootstrap/context"
        );
    }

    /// END-TO-END: drive the REAL rcdzc-compiled PURE-GENESIS reducer (`reducer_pure.cdz`) through the A1
    /// BYTES fold boundary on wasmtime — the smallest Cadenza guest that exercises the whole
    /// `apply(list<u8>) -> list<u8>` round-trip: the kernel `build_event_document`s the Event to bytes, the
    /// guest decodes it, folds, and encodes its effect-list back, and `parse_effect_list` decodes the result.
    /// Where the genesis E2E above drives a KV-writing reducer through the host `seed_genesis` fold path,
    /// THIS reducer is PURE — `fold.apply` only, NO kv import and NO host capabilities — so it proves the A1
    /// bytes boundary END-TO-END with the minimal EFFECT-emitting surface, driving `apply` DIRECTLY.
    /// "PURE" ≠ zero component imports though: reducer_pure.cdz CONSTRUCTS values (structural records +
    /// `String.to-bytes`), so — like any value-building Cadenza reducer — it imports the value-heap runtime
    /// (`cadenza:runtime/heap`, ONE dep), resolved from `CDZ_STORE` exactly as the genesis reducer's deps are.
    ///
    /// The contract (`reducer_pure.cdz`): a PAYLOADED event folds to ONE `emit` effect echoing the payload
    /// (target = the bytes `"out"`, correlation none); a payload-free event folds to no effects.
    ///
    /// Env-gated on `PURE_GENESIS_REDUCER_COMPONENT` ALONE (skip when unset); `CDZ_STORE` is the SHARED store
    /// the agentHostEnvSetup exports for the genesis e2e too, so it may be present before the pure component is
    /// wired — gating on the pair (and fail-loud on a half-wired env) wrongly PANICKED when CDZ_STORE was set
    /// but the pure component absent, which is why cc51998e6 was rejected. So: unset pure component → skip;
    /// pure component set but CDZ_STORE missing → fail loud (the heap dep needs the store). A bare `cargo test`
    /// stays green; v-nix's pure-reducer precompile derivation exports `PURE_GENESIS_REDUCER_COMPONENT` in the
    /// native-check and the e2e runs against the real component. It does NOT go through
    /// `require_reducer_and_store_or_skip` because it gates on ITS OWN component env directly (the genesis/kv
    /// E2Es share that helper only to single-source the reducer-path + CDZ_STORE skip/fail-loud contract).
    #[tokio::test]
    async fn real_pure_reducer_folds_an_event_through_the_a1_bytes_boundary() {
        use cdz_kernel::wasm_host::AsyncComponentReducer;

        // Gate on BOTH the component path AND CDZ_STORE: "PURE" means no kv + no host capabilities, NOT zero
        // component imports — reducer_pure.cdz CONSTRUCTS values (structural records + String.to-bytes to echo
        // the payload), and any value-building Cadenza reducer imports the value-heap runtime
        // (`cadenza:runtime/heap`), which is resolved from the store like the genesis reducer's deps.
        //
        // GATING: skip solely on THIS test's own env, `PURE_GENESIS_REDUCER_COMPONENT`. `CDZ_STORE` is a
        // SHARED resource the nix agentHostEnvSetup exports for the genesis e2e too, so it is often present
        // even when the pure component is NOT yet wired (v-nix lands the PURE_GENESIS_REDUCER_COMPONENT export
        // in a separate MR). Gating on the PAIR (and fail-loud on a half-wired env, as the genesis helper does)
        // was WRONG here: with CDZ_STORE set globally but the pure component absent, it PANICKED
        // (`None, Some`) instead of skipping (the reject on cc51998e6). So: unset pure component → SKIP
        // (regardless of CDZ_STORE); pure component SET but CDZ_STORE missing → fail loud (the pure reducer's
        // heap dep genuinely needs the store, so a set-without-store is a broken pure-genesis wiring).
        let non_empty = |var: &str| std::env::var(var).ok().filter(|v| !v.is_empty());
        let Some(reducer_path) = non_empty("PURE_GENESIS_REDUCER_COMPONENT") else {
            eprintln!(
                "SKIP real_pure_reducer_folds_an_event_through_the_a1_bytes_boundary: \
                 PURE_GENESIS_REDUCER_COMPONENT unset (or empty)"
            );
            return;
        };
        let store_dir = non_empty("CDZ_STORE").unwrap_or_else(|| {
            panic!(
                "real_pure_reducer_...: PURE_GENESIS_REDUCER_COMPONENT is set but CDZ_STORE is not — the pure \
                 reducer imports cadenza:runtime/heap, whose bytes resolve from the component store"
            )
        });
        let bytes = std::fs::read(&reducer_path).unwrap_or_else(|e| {
            panic!("PURE_GENESIS_REDUCER_COMPONENT={reducer_path:?} set but unreadable: {e}")
        });
        let reducer = AsyncComponentReducer::from_component_bytes(&bytes)
            .unwrap_or_else(|e| panic!("reducer_pure must be a valid component: {e:?}"));
        // Resolve the reducer's declared component deps (the value-heap runtime) from CDZ_STORE via
        // `get_by_hash` (the production content-addressed reader — same content-verify the fold uses), then
        // attach the store so the §23 compose can resolve the runtime's OWN transitive bare imports by name.
        // Identical to the genesis reducer's dep-resolve path above.
        let store = cdz_kernel::component_store::ComponentStore::open(&store_dir);
        let deps = reducer.deps().to_vec();
        assert!(
            !deps.is_empty(),
            "the pure reducer must declare a cadenza:runtime/heap dep (it constructs values via the heap)"
        );
        let mut resolved = Vec::with_capacity(deps.len());
        for dep in &deps {
            let dep_bytes = store.get_by_hash(&dep.hash).unwrap_or_else(|e| {
                panic!(
                    "CDZ_STORE={store_dir:?} could not resolve pure reducer dep {:?} (hash {}): {e:?}",
                    dep.import_name,
                    dep.hash.to_hex()
                )
            });
            resolved.push((dep.clone(), dep_bytes));
        }
        let reducer = reducer
            .with_resolved_deps(resolved)
            .with_component_store(store);
        // Drive apply across the A1 bytes boundary (kernel encodes the Event to bytes IN, decodes the
        // effect-list bytes OUT).
        let ct = cdz_kernel::event::ContentType {
            family: "message".into(),
            version: 1,
        };
        let (effects, _kv) = reducer
            .apply(
                cdz_kernel::kv::Kv::new(),
                ct,
                Some(b"echo-me".to_vec()),
                None,
            )
            .await
            .expect(
                "the pure reducer folds an event through the A1 bytes boundary without trapping",
            );

        // A payloaded event → exactly one `emit` effect echoing the payload (the reducer_pure.cdz contract),
        // proving the round-trip: family "emit" + target bytes "out" + payload = the echoed input.
        assert_eq!(effects.len(), 1, "a payloaded event folds to one effect");
        assert_eq!(
            effects[0].request.content_type.family, "emit",
            "the folded effect's kind crosses the bytes boundary as the family string"
        );
        assert_eq!(
            effects[0].request.target_str().unwrap(),
            "out",
            "the effect target is the opaque bytes \"out\""
        );
        let echoed = match &effects[0].request.payload {
            Some(cdz_kernel::effect::Payload::Inline(b)) => b.to_vec(),
            other => panic!("expected an inline echoed payload, got {other:?}"),
        };
        assert_eq!(
            echoed, b"echo-me",
            "the emit effect echoes the event payload verbatim through the bytes round-trip"
        );

        // A payload-FREE event folds to NO effects — the other arm of the pure contract, same reducer.
        let ct2 = cdz_kernel::event::ContentType {
            family: "message".into(),
            version: 1,
        };
        let (none_effects, _kv2) = reducer
            .apply(cdz_kernel::kv::Kv::new(), ct2, None, None)
            .await
            .expect("a payload-free event folds without trapping");
        assert!(
            none_effects.is_empty(),
            "a payload-free event requests no effects (the pure fold's empty arm)"
        );
    }

    /// END-TO-END: drive the REAL rcdzc-compiled KV-GENESIS reducer (`reducer_kv.cdz`) through the A1 BYTES
    /// fold boundary on wasmtime, proving the kv HOST IMPORT round-trips the SAME bytes through the host — the
    /// sibling of the pure-genesis E2E, exercising BOTH `kv.put` AND the new `kv.get` option<list<u8>> lift
    /// (rcdzc §3c GAP C) end to end. The pure E2E proves the effect-emitting bytes boundary with NO host cap;
    /// THIS one adds the `cadenza:agent-kernel/kv` host import (served by the kernel `ReducerHost`, backed by
    /// the passed-in `Kv`) and proves put-then-get returns what put wrote.
    ///
    /// The contract (`reducer_kv.cdz`, v-agent-harness-host-agreed): a `kv-seed`-family event WITH a payload
    /// folds to `kv.put("kv-genesis/slot", payload)` + NO effects; a `kv-read`-family event folds to
    /// `kv.get("kv-genesis/slot")` → on `Some(got)` one `emit` effect echoing the stored bytes (target "out",
    /// correlation none), on `None()` no effects; any other family folds to nothing. Driving SEED then TRIGGER
    /// and asserting the emit payload equals the seeded bytes proves the whole kv put→get round-trip.
    ///
    /// The KV is THREADED across the two folds (the seed fold's returned `Kv` is fed into the trigger fold) —
    /// this is exactly the kernel loop's fold contract (KV moves through each fold), so the trigger's `kv.get`
    /// reads what the seed's `kv.put` committed. Env-gated on `CDZ_KV_GENESIS_REDUCER_COMPONENT` ALONE (skip
    /// when unset); `CDZ_STORE` required only once the component is set (fail loud on half-wired) — the SAME
    /// gating shape as the pure-genesis E2E (the kv reducer also imports `cadenza:runtime/heap` to build its
    /// structural records, resolved from the store). A bare `cargo test` stays green; v-nix's kv-reducer
    /// precompile derivation exports the env in the native-check and this runs against the real component.
    #[tokio::test]
    async fn real_kv_reducer_round_trips_stored_bytes_through_the_kv_host_import() {
        use cdz_kernel::wasm_host::AsyncComponentReducer;

        // Gate on THIS test's own component env; require CDZ_STORE only when the component is set (its heap dep
        // needs the store) — same skip/fail-loud shape as the pure-genesis E2E (CDZ_STORE is shared, so gating
        // on the pair would wrongly panic when the store is present but the kv component isn't yet wired).
        let non_empty = |var: &str| std::env::var(var).ok().filter(|v| !v.is_empty());
        let Some(reducer_path) = non_empty("CDZ_KV_GENESIS_REDUCER_COMPONENT") else {
            eprintln!(
                "SKIP real_kv_reducer_round_trips_stored_bytes_through_the_kv_host_import: \
                 CDZ_KV_GENESIS_REDUCER_COMPONENT unset (or empty)"
            );
            return;
        };
        let store_dir = non_empty("CDZ_STORE").unwrap_or_else(|| {
            panic!(
                "real_kv_reducer_...: CDZ_KV_GENESIS_REDUCER_COMPONENT is set but CDZ_STORE is not — the kv \
                 reducer imports cadenza:runtime/heap, whose bytes resolve from the component store"
            )
        });
        let bytes = std::fs::read(&reducer_path).unwrap_or_else(|e| {
            panic!("CDZ_KV_GENESIS_REDUCER_COMPONENT={reducer_path:?} set but unreadable: {e}")
        });
        let reducer = AsyncComponentReducer::from_component_bytes(&bytes)
            .unwrap_or_else(|e| panic!("reducer_kv must be a valid component: {e:?}"));
        // Resolve the reducer's declared component deps (the value-heap runtime) from CDZ_STORE via
        // `get_by_hash` + attach the store — identical to the pure/genesis dep-resolve path.
        let store = cdz_kernel::component_store::ComponentStore::open(&store_dir);
        let deps = reducer.deps().to_vec();
        assert!(
            !deps.is_empty(),
            "the kv reducer must declare a cadenza:runtime/heap dep (it constructs values via the heap)"
        );
        let mut resolved = Vec::with_capacity(deps.len());
        for dep in &deps {
            let dep_bytes = store.get_by_hash(&dep.hash).unwrap_or_else(|e| {
                panic!(
                    "CDZ_STORE={store_dir:?} could not resolve kv reducer dep {:?} (hash {}): {e:?}",
                    dep.import_name,
                    dep.hash.to_hex()
                )
            });
            resolved.push((dep.clone(), dep_bytes));
        }
        let reducer = reducer
            .with_resolved_deps(resolved)
            .with_component_store(store);

        // SEED: a `kv-seed`-family event with a payload → the reducer's kv.put writes it under its fixed slot
        // and returns NO effects. The put lands on the returned Kv (committed on fold success).
        let stored = b"seeded-kv-genesis-value".to_vec();
        let seed_ct = cdz_kernel::event::ContentType {
            family: "kv-seed".into(),
            version: 1,
        };
        let (seed_effects, kv_after_seed) = reducer
            .apply(
                cdz_kernel::kv::Kv::new(),
                seed_ct,
                Some(stored.clone()),
                None,
            )
            .await
            .expect("the kv reducer folds the seed event (kv.put) through the A1 bytes boundary");
        assert!(
            seed_effects.is_empty(),
            "a kv-seed event writes state (kv.put) and requests no effects"
        );

        // TRIGGER: a `kv-read`-family event → the reducer's kv.get reads the slot back and echoes it in ONE
        // emit effect. Thread the SEED fold's returned Kv in (the kernel loop's fold contract — KV moves
        // through each fold), so the trigger's kv.get sees what the seed's kv.put committed.
        let read_ct = cdz_kernel::event::ContentType {
            family: "kv-read".into(),
            version: 1,
        };
        let (read_effects, _kv_after_read) = reducer
            .apply(kv_after_seed, read_ct, None, None)
            .await
            .expect(
                "the kv reducer folds the trigger event (kv.get) through the A1 bytes boundary",
            );

        // Exactly one emit effect whose payload equals the seeded bytes — kv.get read back what kv.put wrote,
        // the SAME bytes round-tripped through the host kv import + the value-form bytes boundary both ways.
        assert_eq!(
            read_effects.len(),
            1,
            "a kv-read event with the slot populated folds to one emit effect"
        );
        assert_eq!(
            read_effects[0].request.content_type.family, "emit",
            "the folded effect's kind crosses the bytes boundary as the family string"
        );
        assert_eq!(
            read_effects[0].request.target_str().unwrap(),
            "out",
            "the emit effect target is the opaque bytes \"out\""
        );
        let echoed = match &read_effects[0].request.payload {
            Some(cdz_kernel::effect::Payload::Inline(b)) => b.to_vec(),
            other => panic!("expected an inline echoed payload, got {other:?}"),
        };
        assert_eq!(
            echoed, stored,
            "the emit effect echoes the SEEDED bytes — kv.get returned exactly what kv.put stored, proving \
             the round-trip through the host kv import"
        );
    }

    /// END-TO-END: drive the REAL `reducer_kv.cdz` kv-DELETE branch through the A1 BYTES boundary, proving
    /// the `kv.delete` BOOL-lift path round-trips through the host — the third kv host op (put + get + delete)
    /// and the only one returning a FLAT SCALAR (`bool`, no retptr, unlike get's `option<bytes>`) that the
    /// reducer branches on. The kv-genesis E2E above proves put+get; THIS proves delete's existed/absent bool.
    ///
    /// The contract (`reducer_kv.cdz` kv-del branch): a `kv-del`-family event folds to `kv.delete("kv-genesis/slot")`
    /// → on `true` (the key EXISTED) ONE emit effect `{kind=emit, target="deleted", payload=None}`; on `false`
    /// (absent) NO effects. Two arms make it non-vacuous: (1) SEED then DELETE → the slot exists → delete
    /// returns true → one "deleted" emit; (2) DELETE with no prior seed → the slot is absent → false → zero
    /// effects. The KV is THREADED across the seed→delete folds (the kernel loop's fold contract), so the
    /// delete sees the seed's committed write. Same component + env as the kv-genesis E2E
    /// (`CDZ_KV_GENESIS_REDUCER_COMPONENT` + `CDZ_STORE`) — it is the SAME reducer_kv component (kv-del was
    /// appended, kv-seed/kv-read byte-intact).
    #[tokio::test]
    async fn real_kv_reducer_delete_branch_emits_iff_the_key_existed() {
        use cdz_kernel::wasm_host::AsyncComponentReducer;

        let non_empty = |var: &str| std::env::var(var).ok().filter(|v| !v.is_empty());
        let Some(reducer_path) = non_empty("CDZ_KV_GENESIS_REDUCER_COMPONENT") else {
            eprintln!(
                "SKIP real_kv_reducer_delete_branch_emits_iff_the_key_existed: \
                 CDZ_KV_GENESIS_REDUCER_COMPONENT unset (or empty)"
            );
            return;
        };
        let store_dir = non_empty("CDZ_STORE").unwrap_or_else(|| {
            panic!(
                "real_kv_reducer_delete_...: CDZ_KV_GENESIS_REDUCER_COMPONENT is set but CDZ_STORE is not — \
                 the kv reducer imports cadenza:runtime/heap, whose bytes resolve from the component store"
            )
        });
        let bytes = std::fs::read(&reducer_path).unwrap_or_else(|e| {
            panic!("CDZ_KV_GENESIS_REDUCER_COMPONENT={reducer_path:?} set but unreadable: {e}")
        });
        let reducer = AsyncComponentReducer::from_component_bytes(&bytes)
            .unwrap_or_else(|e| panic!("reducer_kv must be a valid component: {e:?}"));
        // Resolve the value-heap runtime dep from CDZ_STORE + attach the store — same path as kv-genesis.
        let store = cdz_kernel::component_store::ComponentStore::open(&store_dir);
        let deps = reducer.deps().to_vec();
        assert!(
            !deps.is_empty(),
            "the kv reducer must declare a cadenza:runtime/heap dep (it constructs values via the heap)"
        );
        let mut resolved = Vec::with_capacity(deps.len());
        for dep in &deps {
            let dep_bytes = store.get_by_hash(&dep.hash).unwrap_or_else(|e| {
                panic!(
                    "CDZ_STORE={store_dir:?} could not resolve kv reducer dep {:?} (hash {}): {e:?}",
                    dep.import_name,
                    dep.hash.to_hex()
                )
            });
            resolved.push((dep.clone(), dep_bytes));
        }
        let reducer = reducer
            .with_resolved_deps(resolved)
            .with_component_store(store);

        // ARM 1 (key EXISTED): SEED the slot, then DELETE it — thread the seed fold's Kv into the delete fold
        // so the delete sees the committed write. delete returns true → one "deleted" emit, no payload.
        let seed_ct = cdz_kernel::event::ContentType {
            family: "kv-seed".into(),
            version: 1,
        };
        let (_seed_effects, kv_after_seed) = reducer
            .apply(
                cdz_kernel::kv::Kv::new(),
                seed_ct,
                Some(b"to-be-deleted".to_vec()),
                None,
            )
            .await
            .expect("the kv reducer folds the seed event (kv.put) through the A1 bytes boundary");
        let del_ct = cdz_kernel::event::ContentType {
            family: "kv-del".into(),
            version: 1,
        };
        let (del_effects, _kv_after_del) = reducer
            .apply(kv_after_seed, del_ct.clone(), None, None)
            .await
            .expect(
                "the kv reducer folds the delete event (kv.delete) through the A1 bytes boundary",
            );
        assert_eq!(
            del_effects.len(),
            1,
            "deleting an EXISTING key folds to one emit (kv.delete returned true)"
        );
        assert_eq!(
            del_effects[0].request.content_type.family, "emit",
            "the folded effect's kind crosses the bytes boundary as the family string"
        );
        assert_eq!(
            del_effects[0].request.target_str().unwrap(),
            "deleted",
            "the delete emit's target is the opaque bytes \"deleted\""
        );
        assert!(
            del_effects[0].request.payload.is_none(),
            "the delete emit carries no payload (the kv-del branch emits target-only)"
        );

        // ARM 2 (key ABSENT): DELETE with no prior seed — the slot doesn't exist → kv.delete returns false →
        // zero effects. Proves the false arm of the bool lift (not just the true arm).
        let (absent_del_effects, _kv3) = reducer
            .apply(cdz_kernel::kv::Kv::new(), del_ct, None, None)
            .await
            .expect("the kv reducer folds a delete-of-absent-key without trapping");
        assert!(
            absent_del_effects.is_empty(),
            "deleting an ABSENT key requests no effects (kv.delete returned false)"
        );
    }

    /// END-TO-END: drive the REAL `reducer_kv.cdz` kv-SCAN branch through the A1 BYTES boundary, proving the
    /// `kv.prefix-scan` LIST-OF-PAIRS lift round-trips through the host — the fourth and richest kv host op
    /// (put + get + delete + prefix-scan), returning `list<tuple<list<u8>, list<u8>>>` (a value-heap List of
    /// Tuple of Bytes the guest forces with `List.len`), the heaviest host result shape of the four. This
    /// completes the put/get/delete/prefix-scan E2E coverage of the kv host op surface.
    ///
    /// The contract (`reducer_kv.cdz` kv-scan branch): a `kv-scan`-family event folds to
    /// `kv.prefix-scan("kv-genesis/")` (the slot's namespace — the seeded `"kv-genesis/slot"` key lives under
    /// it) → on a NON-EMPTY list ONE emit `{kind=emit, target="scanned", payload=None}`; on empty NO effects.
    /// Two arms: (1) SEED the slot then SCAN → the prefix has one pair → one "scanned" emit; (2) SCAN with
    /// nothing seeded → empty → zero effects. KV threaded across seed→scan (the kernel loop's fold contract).
    /// Same component + env as the other kv E2Es (`CDZ_KV_GENESIS_REDUCER_COMPONENT` + `CDZ_STORE`) — the SAME
    /// reducer_kv component (kv-scan appended; kv-seed/kv-read/kv-del byte-intact).
    #[tokio::test]
    async fn real_kv_reducer_scan_branch_emits_iff_the_prefix_is_non_empty() {
        use cdz_kernel::wasm_host::AsyncComponentReducer;

        let non_empty = |var: &str| std::env::var(var).ok().filter(|v| !v.is_empty());
        let Some(reducer_path) = non_empty("CDZ_KV_GENESIS_REDUCER_COMPONENT") else {
            eprintln!(
                "SKIP real_kv_reducer_scan_branch_emits_iff_the_prefix_is_non_empty: \
                 CDZ_KV_GENESIS_REDUCER_COMPONENT unset (or empty)"
            );
            return;
        };
        let store_dir = non_empty("CDZ_STORE").unwrap_or_else(|| {
            panic!(
                "real_kv_reducer_scan_...: CDZ_KV_GENESIS_REDUCER_COMPONENT is set but CDZ_STORE is not — \
                 the kv reducer imports cadenza:runtime/heap, whose bytes resolve from the component store"
            )
        });
        let bytes = std::fs::read(&reducer_path).unwrap_or_else(|e| {
            panic!("CDZ_KV_GENESIS_REDUCER_COMPONENT={reducer_path:?} set but unreadable: {e}")
        });
        let reducer = AsyncComponentReducer::from_component_bytes(&bytes)
            .unwrap_or_else(|e| panic!("reducer_kv must be a valid component: {e:?}"));
        // Resolve the value-heap runtime dep from CDZ_STORE + attach the store — same path as the other kv E2Es.
        let store = cdz_kernel::component_store::ComponentStore::open(&store_dir);
        let deps = reducer.deps().to_vec();
        assert!(
            !deps.is_empty(),
            "the kv reducer must declare a cadenza:runtime/heap dep (it constructs values via the heap)"
        );
        let mut resolved = Vec::with_capacity(deps.len());
        for dep in &deps {
            let dep_bytes = store.get_by_hash(&dep.hash).unwrap_or_else(|e| {
                panic!(
                    "CDZ_STORE={store_dir:?} could not resolve kv reducer dep {:?} (hash {}): {e:?}",
                    dep.import_name,
                    dep.hash.to_hex()
                )
            });
            resolved.push((dep.clone(), dep_bytes));
        }
        let reducer = reducer
            .with_resolved_deps(resolved)
            .with_component_store(store);

        // ARM 1 (prefix NON-empty): SEED the slot, then SCAN its namespace — thread the seed fold's Kv into
        // the scan fold so the scan sees the committed pair. prefix-scan returns a one-element list → one
        // "scanned" emit, no payload.
        let seed_ct = cdz_kernel::event::ContentType {
            family: "kv-seed".into(),
            version: 1,
        };
        let (_seed_effects, kv_after_seed) = reducer
            .apply(
                cdz_kernel::kv::Kv::new(),
                seed_ct,
                Some(b"scannable-value".to_vec()),
                None,
            )
            .await
            .expect("the kv reducer folds the seed event (kv.put) through the A1 bytes boundary");
        let scan_ct = cdz_kernel::event::ContentType {
            family: "kv-scan".into(),
            version: 1,
        };
        let (scan_effects, _kv_after_scan) = reducer
            .apply(kv_after_seed, scan_ct.clone(), None, None)
            .await
            .expect("the kv reducer folds the scan event (kv.prefix-scan) through the A1 bytes boundary");
        assert_eq!(
            scan_effects.len(),
            1,
            "scanning a NON-EMPTY prefix folds to one emit (prefix-scan returned a non-empty list of pairs)"
        );
        assert_eq!(
            scan_effects[0].request.content_type.family, "emit",
            "the folded effect's kind crosses the bytes boundary as the family string"
        );
        assert_eq!(
            scan_effects[0].request.target_str().unwrap(),
            "scanned",
            "the scan emit's target is the opaque bytes \"scanned\""
        );
        assert!(
            scan_effects[0].request.payload.is_none(),
            "the scan emit carries no payload (the kv-scan branch emits target-only)"
        );

        // ARM 2 (prefix EMPTY): SCAN with nothing seeded — the prefix has no pairs → prefix-scan returns an
        // empty list → zero effects. Proves the empty arm of the list-of-pairs lift (List.len == 0 branch).
        let (empty_scan_effects, _kv3) = reducer
            .apply(cdz_kernel::kv::Kv::new(), scan_ct, None, None)
            .await
            .expect("the kv reducer folds a scan-of-empty-prefix without trapping");
        assert!(
            empty_scan_effects.is_empty(),
            "scanning an EMPTY prefix requests no effects (prefix-scan returned an empty list)"
        );
    }

    /// END-TO-END: drive the REAL `reducer_agent_loop.cdz` — the GAP-1 KEYSTONE — through the A1 BYTES
    /// boundary on wasmtime, proving the harness hosts a REAL AGENTIC LOOP as a pure reducer: the CLOSED
    /// message→model→tool→result→model loop, with the inbox + growing context managed as durable KV state
    /// enumerated via kv.prefix-scan (the loop a self-hosting agent runs instead of tmux/fleet-tooling state).
    /// This is the highest-signal proof of the whole self-hosting-harness arc: no new kernel mechanism, just
    /// a fold over the existing kv + emit effects (v-agent-harness-host's greenlit recommendation).
    ///
    /// The contract (`reducer_agent_loop.cdz`, spine `7432bb96a` + loop-close `3b58ceb66`): a `message` event
    /// with a payload appends it to the `inbox/` prefix (kv.put) then, iff the inbox prefix-scan is non-empty,
    /// emits ONE `model` effect (target "llm") — read-inbox→call-model. A `model-response` event emits ONE
    /// `tool` effect (target "shell") — the action step. A `tool-result` event folds the result into the
    /// `context/` prefix then, iff the context prefix-scan is non-empty, RE-INVOKES the model — ONE more
    /// `model` effect (target "llm") — CLOSING the loop. Anything else folds to nothing. Each arm's payload
    /// echoes the driving event's, and the KV is threaded across all three folds (the kernel loop's fold
    /// contract) so the inbox/context accumulate exactly as a live session's would.
    ///
    /// Env-gated on `CDZ_AGENT_LOOP_REDUCER_COMPONENT` (skip when unset); `CDZ_STORE` required once the
    /// component is set (the reducer imports `cadenza:runtime/heap`) — the SAME skip/fail-loud shape as the
    /// kv-genesis E2E. A bare `cargo test` stays green; v-nix's reducerCadenzaAgentLoop precompile derivation
    /// exports the env in the native-check and this runs against the real component.
    #[tokio::test]
    async fn real_agent_loop_reducer_folds_the_closed_message_model_tool_result_loop() {
        use cdz_kernel::wasm_host::AsyncComponentReducer;

        let non_empty = |var: &str| std::env::var(var).ok().filter(|v| !v.is_empty());
        let Some(reducer_path) = non_empty("CDZ_AGENT_LOOP_REDUCER_COMPONENT") else {
            eprintln!(
                "SKIP real_agent_loop_reducer_folds_the_closed_message_model_tool_result_loop: \
                 CDZ_AGENT_LOOP_REDUCER_COMPONENT unset (or empty)"
            );
            return;
        };
        let store_dir = non_empty("CDZ_STORE").unwrap_or_else(|| {
            panic!(
                "real_agent_loop_reducer_...: CDZ_AGENT_LOOP_REDUCER_COMPONENT is set but CDZ_STORE is not — \
                 the agent-loop reducer imports cadenza:runtime/heap, whose bytes resolve from the store"
            )
        });
        let bytes = std::fs::read(&reducer_path).unwrap_or_else(|e| {
            panic!("CDZ_AGENT_LOOP_REDUCER_COMPONENT={reducer_path:?} set but unreadable: {e}")
        });
        let reducer = AsyncComponentReducer::from_component_bytes(&bytes)
            .unwrap_or_else(|e| panic!("reducer_agent_loop must be a valid component: {e:?}"));
        // Resolve the value-heap runtime dep from CDZ_STORE + attach the store — same path as the kv E2Es.
        let store = cdz_kernel::component_store::ComponentStore::open(&store_dir);
        let deps = reducer.deps().to_vec();
        assert!(
            !deps.is_empty(),
            "the agent-loop reducer must declare a cadenza:runtime/heap dep (it constructs values via the heap)"
        );
        let mut resolved = Vec::with_capacity(deps.len());
        for dep in &deps {
            let dep_bytes = store.get_by_hash(&dep.hash).unwrap_or_else(|e| {
                panic!(
                    "CDZ_STORE={store_dir:?} could not resolve agent-loop reducer dep {:?} (hash {}): {e:?}",
                    dep.import_name,
                    dep.hash.to_hex()
                )
            });
            resolved.push((dep.clone(), dep_bytes));
        }
        let reducer = reducer
            .with_resolved_deps(resolved)
            .with_component_store(store);

        // STEP 1 (read-inbox → call-model): a `message` event appends to the inbox and, since the inbox is
        // now non-empty, emits ONE `model` effect targeting the LLM with the message as context. This is the
        // agentic loop's "the agent read its inbox and decided to call the model" step.
        let message = b"summarize the build log".to_vec();
        let msg_ct = cdz_kernel::event::ContentType {
            family: "message".into(),
            version: 1,
        };
        let (msg_effects, kv_after_msg) = reducer
            .apply(
                cdz_kernel::kv::Kv::new(),
                msg_ct,
                Some(message.clone()),
                None,
            )
            .await
            .expect("the agent-loop reducer folds a message event through the A1 bytes boundary");
        assert_eq!(
            msg_effects.len(),
            1,
            "a message event with a non-empty inbox folds to one model effect (read-inbox → call-model)"
        );
        assert_eq!(
            msg_effects[0].request.content_type.family, "model",
            "the message step emits a `model` effect (invoke the LLM)"
        );
        assert_eq!(
            msg_effects[0].request.target_str().unwrap(),
            "llm",
            "the model effect targets the opaque bytes \"llm\""
        );
        // The model-effect payload is a value-form M1 `ModelRequest` doc (b1: the reducer emits a structured
        // ModelRequest, NOT the raw message bytes). Decode it and assert on the request STRUCTURE — the model
        // id the transport gates on + that the enumerated conversation carries the message as a user Text
        // turn. This is what my converse.rs `from_model_request` consumes downstream (the decode is the same
        // codec, now value-form after v-cml's collapse 616f9080f).
        let model_payload = match &msg_effects[0].request.payload {
            Some(cdz_kernel::effect::Payload::Inline(b)) => b.to_vec(),
            other => panic!("expected an inline model-effect payload, got {other:?}"),
        };
        let m1 = cdz_kernel::event_ast::decode_model_request(&model_payload)
            .expect("the model effect's payload decodes as a value-form M1 ModelRequest");
        assert_eq!(
            m1.model, "claude",
            "the M1 request names the model the transport invokes"
        );
        assert!(
            !m1.messages.is_empty(),
            "the M1 request carries the enumerated inbox conversation (non-empty)"
        );
        let carries_message = m1.messages.iter().any(|turn| {
            turn.content.iter().any(|blk| {
                matches!(blk, cdz_kernel::event_ast::ContentBlock::Text(t)
                    if t.as_bytes() == message.as_slice())
            })
        });
        assert!(
            carries_message,
            "the M1 conversation carries the message as a Text content block (enumerated from the inbox)"
        );

        // STEP 2 (action): a `model-response` event (the LLM asked for a tool call) folds to ONE `tool`
        // effect targeting the shell with the call — the loop's ACTION step. Thread the post-message KV in
        // (the loop's fold contract), though this arm reads no inbox state.
        let call = b"cargo test --lib".to_vec();
        let resp_ct = cdz_kernel::event::ContentType {
            family: "model-response".into(),
            version: 1,
        };
        let (resp_effects, kv_after_resp) = reducer
            .apply(kv_after_msg, resp_ct, Some(call.clone()), None)
            .await
            .expect(
                "the agent-loop reducer folds a model-response event through the A1 bytes boundary",
            );
        assert_eq!(
            resp_effects.len(),
            1,
            "a model-response event folds to one tool effect (the loop's action step)"
        );
        assert_eq!(
            resp_effects[0].request.content_type.family, "tool",
            "the model-response step emits a `tool` effect (dispatch the requested tool call)"
        );
        assert_eq!(
            resp_effects[0].request.target_str().unwrap(),
            "shell",
            "the tool effect targets the opaque bytes \"shell\""
        );
        let tool_payload = match &resp_effects[0].request.payload {
            Some(cdz_kernel::effect::Payload::Inline(b)) => b.to_vec(),
            other => panic!("expected an inline tool-effect payload, got {other:?}"),
        };
        assert_eq!(
            tool_payload, call,
            "the tool effect carries the model's tool-call request verbatim"
        );

        // STEP 3 (loop CLOSURE): a `tool-result` event folds the tool's result into the "context/" working
        // set (kv.put) then, since the context prefix-scan is now non-empty, RE-INVOKES the model — emitting
        // ONE more `model` effect. This is what makes it a LOOP, not a single pass: message→model→tool→
        // RESULT→model→… Thread the post-response KV in so the context accumulates across the whole cycle.
        let result = b"exit 0: 42 tests passed".to_vec();
        let tool_result_ct = cdz_kernel::event::ContentType {
            family: "tool-result".into(),
            version: 1,
        };
        let (result_effects, _kv_after_result) = reducer
            .apply(kv_after_resp, tool_result_ct, Some(result.clone()), None)
            .await
            .expect(
                "the agent-loop reducer folds a tool-result event through the A1 bytes boundary",
            );
        assert_eq!(
            result_effects.len(),
            1,
            "a tool-result event RE-INVOKES the model (loop closure): one more model effect"
        );
        assert_eq!(
            result_effects[0].request.content_type.family, "model",
            "the tool-result step emits another `model` effect (re-invoke the LLM with the grown context)"
        );
        assert_eq!(
            result_effects[0].request.target_str().unwrap(),
            "llm",
            "the re-invoke model effect targets the opaque bytes \"llm\""
        );
        // The re-invoke model effect's payload is also a value-form M1 ModelRequest (b1). Decode it and assert
        // the grown conversation carries the tool result as a ToolResult content block — closing
        // message→model→tool→result→model with the tool output folded into the context via kv.prefix-scan.
        let reinvoke_payload = match &result_effects[0].request.payload {
            Some(cdz_kernel::effect::Payload::Inline(b)) => b.to_vec(),
            other => panic!("expected an inline re-invoke model-effect payload, got {other:?}"),
        };
        let m1_reinvoke = cdz_kernel::event_ast::decode_model_request(&reinvoke_payload)
            .expect("the re-invoke model effect's payload decodes as a value-form M1 ModelRequest");
        assert_eq!(m1_reinvoke.model, "claude");
        let carries_result = m1_reinvoke.messages.iter().any(|turn| {
            turn.content.iter().any(|blk| {
                matches!(blk, cdz_kernel::event_ast::ContentBlock::ToolResult { result: r, .. }
                    if r.as_slice() == result.as_slice())
            })
        });
        assert!(
            carries_result,
            "the re-invoke M1 conversation carries the tool result as a ToolResult content block — closing \
             message→model→tool→result→model end-to-end through the host (context grown via kv.prefix-scan)"
        );

        // NEGATIVE: an unrelated event family is not part of the loop → no effects.
        let other_ct = cdz_kernel::event::ContentType {
            family: "unrelated".into(),
            version: 1,
        };
        let (other_effects, _kv3) = reducer
            .apply(
                cdz_kernel::kv::Kv::new(),
                other_ct,
                Some(b"noise".to_vec()),
                None,
            )
            .await
            .expect("the agent-loop reducer folds an unrelated event without trapping");
        assert!(
            other_effects.is_empty(),
            "an event family outside the loop requests no effects"
        );
    }

    /// END-TO-END: the agent-loop reducer ACCUMULATES its inbox across multiple messages in durable KV,
    /// enumerated via kv.prefix-scan — the distinctive "self-hosting agent manages its own inbox/context in
    /// durable KV rather than tmux/fleet-tooling state" property. The closed-loop E2E above proves the
    /// message→model→tool→result→model spine with a SINGLE inbox entry; THIS proves the inbox is genuinely
    /// STATEFUL: two distinct messages, threaded through the fold KV (the kernel loop's contract), leave TWO
    /// pairs under the `inbox/` prefix that prefix-scan enumerates. Asserts on the KV STATE + effect
    /// family/target, NOT the model-effect payload bytes — so it is independent of the M1 structured-payload
    /// change (which reshapes only the payload, not the inbox accumulation).
    ///
    /// Same component + env as the other agent-loop E2E (`CDZ_AGENT_LOOP_REDUCER_COMPONENT` + `CDZ_STORE`).
    #[tokio::test]
    async fn real_agent_loop_reducer_accumulates_its_inbox_across_messages_in_kv() {
        use cdz_kernel::wasm_host::AsyncComponentReducer;

        let non_empty = |var: &str| std::env::var(var).ok().filter(|v| !v.is_empty());
        let Some(reducer_path) = non_empty("CDZ_AGENT_LOOP_REDUCER_COMPONENT") else {
            eprintln!(
                "SKIP real_agent_loop_reducer_accumulates_its_inbox_across_messages_in_kv: \
                 CDZ_AGENT_LOOP_REDUCER_COMPONENT unset (or empty)"
            );
            return;
        };
        let store_dir = non_empty("CDZ_STORE").unwrap_or_else(|| {
            panic!(
                "real_agent_loop_reducer_accumulates_...: CDZ_AGENT_LOOP_REDUCER_COMPONENT is set but \
                 CDZ_STORE is not — the agent-loop reducer imports cadenza:runtime/heap, resolved from the store"
            )
        });
        let bytes = std::fs::read(&reducer_path).unwrap_or_else(|e| {
            panic!("CDZ_AGENT_LOOP_REDUCER_COMPONENT={reducer_path:?} set but unreadable: {e}")
        });
        let reducer = AsyncComponentReducer::from_component_bytes(&bytes)
            .unwrap_or_else(|e| panic!("reducer_agent_loop must be a valid component: {e:?}"));
        let store = cdz_kernel::component_store::ComponentStore::open(&store_dir);
        let deps = reducer.deps().to_vec();
        assert!(
            !deps.is_empty(),
            "the agent-loop reducer must declare a cadenza:runtime/heap dep (it constructs values via the heap)"
        );
        let mut resolved = Vec::with_capacity(deps.len());
        for dep in &deps {
            let dep_bytes = store.get_by_hash(&dep.hash).unwrap_or_else(|e| {
                panic!(
                    "CDZ_STORE={store_dir:?} could not resolve agent-loop reducer dep {:?} (hash {}): {e:?}",
                    dep.import_name,
                    dep.hash.to_hex()
                )
            });
            resolved.push((dep.clone(), dep_bytes));
        }
        let reducer = reducer
            .with_resolved_deps(resolved)
            .with_component_store(store);

        // Deliver TWO distinct messages, threading the fold KV through both (message A then message B). Each
        // message appends to the "inbox/" prefix and — the inbox being non-empty — emits one model effect.
        let msg_a = b"first task: read the log".to_vec();
        let msg_b = b"second task: summarize it".to_vec();
        let msg_ct = || cdz_kernel::event::ContentType {
            family: "message".into(),
            version: 1,
        };

        let (effects_a, kv_after_a) = reducer
            .apply(
                cdz_kernel::kv::Kv::new(),
                msg_ct(),
                Some(msg_a.clone()),
                None,
            )
            .await
            .expect("the agent-loop reducer folds the first message through the A1 bytes boundary");
        assert_eq!(
            effects_a.len(),
            1,
            "the first message (inbox now non-empty) folds to one model effect"
        );
        assert_eq!(effects_a[0].request.content_type.family, "model");

        let (effects_b, kv_after_b) = reducer
            .apply(kv_after_a, msg_ct(), Some(msg_b.clone()), None)
            .await
            .expect(
                "the agent-loop reducer folds the second message through the A1 bytes boundary",
            );
        assert_eq!(
            effects_b.len(),
            1,
            "the second message also folds to one model effect (inbox still non-empty)"
        );
        assert_eq!(effects_b[0].request.content_type.family, "model");

        // THE ACCUMULATION PROOF: after both folds, the reducer's own `inbox/` prefix holds BOTH messages'
        // pairs — the inbox is durable, stateful KV the reducer grows and re-enumerates, not per-turn scratch.
        // (The keys are `inbox/` ++ the message bytes, per reducer_agent_loop.cdz's `inbox-key`.)
        let inbox = kv_after_b.prefix_scan(b"inbox/");
        assert_eq!(
            inbox.len(),
            2,
            "both messages accumulated under the inbox/ prefix (durable stateful inbox, enumerated via \
             prefix-scan) — got {} entries",
            inbox.len()
        );
        let values: std::collections::BTreeSet<Vec<u8>> =
            inbox.iter().map(|(_k, v)| v.to_vec()).collect();
        assert!(
            values.contains(&msg_a) && values.contains(&msg_b),
            "the accumulated inbox holds BOTH distinct messages' payloads"
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
        let id = SessionId::new(Hash::of(b"durable"));
        host.spawn(id, now_host().with_sink(Box::new(sink)));
        // Drive a turn — the session appends events (Inbound + the Now dispatch/result), each written
        // through to the sink. Assert the turn actually SUCCEEDED (Some(Ok)) — a KernelError turn would
        // still append the Inbound, so a log-length-only check could pass on a failed turn (#1988 review).
        assert!(
            matches!(host.deliver(&id, inbound_go(), None).await, Some(Ok(()))),
            "the durable session ran its turn without a kernel error"
        );

        // Recovering the durable file replays every event appended AFTER the sink was attached. The sink is
        // attached post-genesis (with_sink is a builder over genesis), so the Genesis event predates it and
        // isn't persisted through this sink — the durable log holds the turn's later events (log-decouple I5:
        // there is no resident Vec to length-compare against; assert on the durable SOURCE, which is what a
        // real recovery reads). A Now turn appends the Inbound + the Now Dispatched + its EffectResult.
        let recovered = LogStore::recover(&path).expect("recover the durable log");
        assert!(
            !recovered.events.is_empty(),
            "the turn's events reached durable storage"
        );
        assert!(
            recovered.events.len() >= 3,
            "the durable log holds the turn's post-genesis events (Inbound + Now dispatch + result), got {}",
            recovered.events.len()
        );
        assert!(
            recovered
                .events
                .iter()
                .any(|e| matches!(e.body, EventBody::Inbound { .. })),
            "the turn's Inbound reached durable storage"
        );
        // Clean up the unique per-run dir (best-effort — the process is ending; a leftover unique dir can't
        // poison another run since the pid+seq is distinct each time).
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn recovery_from_the_durable_log_reconstructs_the_identical_kv_state() {
        // THE load-bearing log-decouple invariant (I5): with the resident log Vec GONE, a session's state is
        // reconstructed SOLELY from its durable log SOURCE on recovery — so replaying the durable log through
        // the same reducer must yield byte-identical state to the live session. This is recovery-equivalence:
        // the property the whole "log is host-cold, kernel keeps only derived state" close rests on. Drive a
        // real Now turn under a recording sink (the durable SOURCE, genesis-seeded), then replay from it.
        use cdz_kernel::kernel::Session;

        // Build the session, then attach a recording sink via the with_sink builder — SEEDED with the
        // session's genesis so the captured buffer is the COMPLETE durable log (with_sink attaches
        // post-genesis, like a production durable sink; replay needs the genesis at events[0]). The recording
        // sink IS the durable log source, exactly what LogStore::recover reads back in production, but hermetic.
        let base = now_host();
        let genesis = base.session().genesis_ref().clone();
        let (sink, captured) = crate::testutil::log_capture::recording_sink_seeded(genesis);
        let mut hosted = base.with_sink(sink);
        hosted.deliver(inbound_go(), None).await.unwrap();

        // The live session ran its turn (ClockAgent: inbound → Now → records "ran").
        let live_kv_root = hosted.session().snapshot().kv_root;
        assert_eq!(
            hosted.session().kv().get(b"status").as_deref(),
            Some(&b"ran"[..]),
            "the live session folded its turn to completion"
        );

        // RECOVER: replay the durable log source through a fresh reducer — no executor consulted (the recorded
        // Now EffectResult supplies the instant), no resident Vec read. The reconstructed state must match.
        let recovered = Session::replay(
            crate::testutil::log_capture::replay_input(&captured),
            &mut ClockAgent,
        )
        .await
        .expect("the durable log replays cleanly");
        assert_eq!(
            recovered.kv().get(b"status").as_deref(),
            Some(&b"ran"[..]),
            "recovery from the durable log reconstructs the KV the live turn produced"
        );
        assert_eq!(
            recovered.snapshot().kv_root,
            live_kv_root,
            "recovery-equivalence: durable-log replay yields byte-identical KV (kv_root) to the live session"
        );
        assert_eq!(
            recovered.genesis_hash(),
            hosted.genesis_hash(),
            "recovery keeps the same id (the nonce is read from the log's genesis, never re-minted)"
        );
    }

    #[tokio::test]
    async fn delivering_to_an_unknown_session_is_none_not_a_panic() {
        let mut host = AgentHost::new();
        // No session registered → None (an unknown id is distinct from a loop error).
        assert!(host
            .deliver(&SessionId::new(Hash::of(b"nope")), inbound_go(), None)
            .await
            .is_none());
        assert!(host.get(&SessionId::new(Hash::of(b"nope"))).is_none());
    }

    #[test]
    fn registry_lists_and_removes_sessions() {
        let mut host = AgentHost::new();
        host.spawn(SessionId::new(Hash::of(b"b")), now_host());
        host.spawn(SessionId::new(Hash::of(b"a")), now_host());
        // Listed sorted deterministically by SessionId (= genesis-hash byte order), independent of insertion
        // order. Build the expected vec by the SAME sort so it pins ordering without hardcoding hash bytes.
        let mut expected = vec![
            SessionId::new(Hash::of(b"a")),
            SessionId::new(Hash::of(b"b")),
        ];
        expected.sort();
        assert_eq!(host.session_ids(), expected);
        // Remove one → gone.
        assert!(host.remove(&SessionId::new(Hash::of(b"a"))).is_some());
        assert!(!host.contains(&SessionId::new(Hash::of(b"a"))));
        assert_eq!(host.len(), 1);
        // Removing an absent id is None, not a panic.
        assert!(host.remove(&SessionId::new(Hash::of(b"a"))).is_none());
    }

    #[tokio::test]
    async fn host_metrics_record_at_the_lifecycle_and_turn_boundaries() {
        // The metric surface records at the host boundaries: spawn (install), deliver (turn ok/err +
        // unknown-session), remove — into the registry. Registry Counters are drain-on-report with no value
        // getter, so drive a real sequence (must not panic) + assert the registry reports over the recorded
        // metrics (the export path). The per-boundary increment logic is exercised; the values reach the
        // exporter, not a test getter.
        let mut host = AgentHost::new();
        host.spawn(SessionId::new(Hash::of(b"a")), now_host());
        host.spawn(SessionId::new(Hash::of(b"b")), now_host());
        // A delivered turn to a known session (the `now` reducer completes Ok).
        host.deliver(&SessionId::new(Hash::of(b"a")), inbound_go(), None)
            .await;
        // A delivery to an UNKNOWN id — recorded distinctly (deliveries_to_unknown_session), not as a turn.
        host.deliver(&SessionId::new(Hash::of(b"ghost")), inbound_go(), None)
            .await;
        // Remove one.
        host.remove(&SessionId::new(Hash::of(b"b")));

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
        let id = SessionId::new(Hash::of(b"worker"));
        host.spawn(id, now_host());
        host.spawn(id, now_host()); // restart — replaces + records a removal
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
        let id = SessionId::new(Hash::of(b"worker"));
        host.spawn(id, now_host());
        // Drive the first instance to completion → it recorded "ran".
        host.deliver(&id, inbound_go(), None).await;
        assert_eq!(
            host.get(&id)
                .unwrap()
                .session()
                .kv()
                .get(b"status")
                .as_deref(),
            Some(&b"ran"[..])
        );
        assert_eq!(host.len(), 1);

        // Re-spawn a FRESH session under the SAME id (a restart). The old one is dropped, not kept.
        host.spawn(id, now_host());
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
        let id = SessionId::new(Hash::of(b"victim"));
        host.spawn(id, now_host());
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
        let victim_id = SessionId::new(victim_hash);
        host.spawn(victim_id, now_host());

        // Both members present before the death.
        assert_eq!(
            host.canonical_store()
                .unwrap()
                .borrow()
                .resolve_all(GROUP)
                .unwrap(),
            [victim_hash, survivor_hash].into_iter().collect(),
            "both members are in the group before termination"
        );

        // Terminate the victim → I5 scan-on-death retracts it from the group.
        host.terminate(&victim_id, Hash::of(b"ctl"), "kill".into())
            .await
            .expect("victim present")
            .expect("fresh terminate");

        // The victim is evicted; the survivor remains (observed-remove is precise).
        let members = host
            .canonical_store()
            .unwrap()
            .borrow()
            .resolve_all(GROUP)
            .unwrap();
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
    async fn terminate_evicts_the_dead_session_from_all_its_groups_i5_multi_group() {
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
        let victim_id = SessionId::new(victim_hash);
        host.spawn(victim_id, now_host());

        // Present in all three before the death.
        for g in [LOBBY, OPS, SOLO] {
            assert!(
                host.canonical_store()
                    .unwrap()
                    .borrow()
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

        let store = host.canonical_store().unwrap().borrow();
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
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
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
        // test would FAIL against the old `SessionId::new(parent_hash)` lookup (the signal would be
        // silently dropped), which is the regression it pins.
        let supervisor = HostedSession::genesis(
            Hash::of(b"supervisor-v1"),
            Box::new(ChildExitedFoldingReducer),
            Box::new(Authorizer::deny_all()),
            CompositeExecutor::new(),
        );
        let parent = SessionId::new(Hash::of(b"concierge")); // vanity id ≠ hex(genesis_hash)
        assert_ne!(
            parent.to_hex(),
            supervisor.genesis_hash().to_hex(),
            "the parent is deliberately registered under a vanity id, not its genesis-hash hex"
        );
        host.spawn(parent, supervisor);

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
        let child_hash = child_id.hash();

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

    /// §6 supervision test reducer (CHILD): self-completes on its first inbound by returning
    /// [`FoldOutput::close`] with `CloseOutcome::Success` carrying a small result payload — a worker that
    /// finishes its task and hands a value back to its supervisor. Non-inbound events are a no-op.
    struct SelfClosingChildReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for SelfClosingChildReducer {
        async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            if let EventBody::Inbound { .. } = &event.body {
                FoldOutput::close(cdz_kernel::event::CloseOutcome::Success(Payload::Inline(
                    b"done".to_vec().into(),
                )))
            } else {
                FoldOutput::none()
            }
        }
    }

    /// §6 supervision test reducer (PARENT): folds a `lifecycle/child-completed` Inbound, decodes the
    /// canonical codec payload, and records the completed child's hash + a success/failure marker into KV so
    /// the test can observe the host delivered the normal-completion signal. Any other inbound is a no-op.
    struct ChildCompletedFoldingReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for ChildCompletedFoldingReducer {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
            if let EventBody::Inbound {
                content_type,
                payload,
            } = &event.body
            {
                if content_type.matches_family("lifecycle/child-completed") {
                    // Pin the v1 wire contract (family+payload unchanged must not mask a version bump).
                    kv.put(
                        b"completed-version".to_vec(),
                        content_type.version.to_string().into_bytes(),
                    );
                    if let Payload::Inline(bytes) = payload {
                        if let Ok((child, outcome)) =
                            cdz_kernel::ast_marshal::decode_child_completed(bytes)
                        {
                            kv.put(b"completed-child".to_vec(), child.to_hex().into_bytes());
                            let marker = match outcome {
                                cdz_kernel::event::CloseOutcome::Success(_) => b"success".to_vec(),
                                cdz_kernel::event::CloseOutcome::Failure(r) => r.into_bytes(),
                            };
                            kv.put(b"completed-outcome".to_vec(), marker);
                        }
                    }
                }
            }
            FoldOutput::none()
        }
    }

    #[tokio::test]
    async fn a_self_closed_child_delivers_child_completed_into_the_parents_inbox_supervision() {
        // §6 supervision (the normal-close counterpart to the §I7 child-exited path): when a child SELF-CLOSES
        // (its reducer returns FoldOutput::close), the reap drops it from the registry and delivers a
        // `lifecycle/child-completed` Inbound (canonical codec payload) into its PARENT's inbox — the
        // supervisor's completion signal. E2E: spawn a supervisor under a VANITY id (pins the resolve-by-
        // genesis-hash contract), spawn a child under it, deliver an inbound that makes the child self-close
        // Success, reap, and observe the parent folded the child hash + success marker.
        let mut host = AgentHost::new();
        let supervisor = HostedSession::genesis(
            Hash::of(b"supervisor-completed-v1"),
            Box::new(ChildCompletedFoldingReducer),
            Box::new(Authorizer::deny_all()),
            CompositeExecutor::new(),
        );
        let parent = SessionId::new(Hash::of(b"orchestrator")); // vanity id != hex(genesis_hash)
        assert_ne!(
            parent.to_hex(),
            supervisor.genesis_hash().to_hex(),
            "the parent is registered under a vanity id, not its genesis-hash hex"
        );
        host.spawn(parent, supervisor);

        let child_id = host
            .spawn_child(
                &parent,
                Hash::of(b"self-closing-child-v1"),
                Box::new(SelfClosingChildReducer),
                Box::new(Authorizer::deny_all()),
                CompositeExecutor::new(),
            )
            .await
            .expect("parent present")
            .expect("child spawned");
        let child_hash = child_id.hash();

        // Deliver an inbound to the child → its fold returns close(Success) → the kernel appends the terminal
        // Closed event + flips is_closed(); the child LINGERS registered (the host owns removal) until reap.
        host.deliver(
            &child_id,
            EventBody::Inbound {
                content_type: cdz_kernel::event::ContentType {
                    family: "task/go".into(),
                    version: 1,
                },
                payload: Payload::Inline(b"work".to_vec().into()),
            },
            None,
        )
        .await
        .expect("child present")
        .expect("delivered");

        assert!(
            host.get(&child_id).unwrap().session().is_closed(),
            "the child self-closed but is still registered before the reap (kernel doesn't touch the registry)"
        );
        assert!(
            host.get(&parent)
                .unwrap()
                .session()
                .kv()
                .get(b"completed-child")
                .is_none(),
            "no child-completed folded before the reap runs"
        );

        // Reap → drop the closed child from the registry + deliver child-completed into the supervisor.
        host.reap_closed_and_notify().await;

        assert!(
            !host.contains(&child_id),
            "the self-closed child is reaped from the registry"
        );
        let kv = host.get(&parent).unwrap();
        let kv = kv.session().kv();
        assert_eq!(
            kv.get(b"completed-child").map(|v| v.to_vec()),
            Some(child_hash.to_hex().into_bytes()),
            "the parent folded ChildCompleted carrying the self-closed child's hash"
        );
        assert_eq!(
            kv.get(b"completed-outcome").map(|v| v.to_vec()),
            Some(b"success".to_vec()),
            "a self-close Success round-trips to the supervisor via the canonical codec"
        );
        assert_eq!(
            kv.get(b"completed-version").map(|v| v.to_vec()),
            Some(b"1".to_vec()),
            "the ChildCompleted Inbound carries content_type.version == 1 (v1 wire contract)"
        );
    }

    #[tokio::test]
    async fn a_self_closed_root_session_is_reaped_with_no_notify_supervision() {
        // §6 supervision edge (mirrors the §I7 root test): a ROOT session (no parent) that self-closes is
        // just REAPED from the registry — there is no parent to notify, and the reap is a clean no-op on the
        // notify path (no panic, no stray delivery).
        let mut host = AgentHost::new();
        let root = HostedSession::genesis(
            Hash::of(b"root-self-close-v1"),
            Box::new(SelfClosingChildReducer),
            Box::new(Authorizer::deny_all()),
            CompositeExecutor::new(),
        );
        let root_id = SessionId::new(root.genesis_hash());
        assert!(
            root.session().parent().is_none(),
            "a genesis session with no spawn edge is a root (parent == None)"
        );
        host.spawn(root_id, root);

        host.deliver(
            &root_id,
            EventBody::Inbound {
                content_type: cdz_kernel::event::ContentType {
                    family: "task/go".into(),
                    version: 1,
                },
                payload: Payload::Inline(b"work".to_vec().into()),
            },
            None,
        )
        .await
        .expect("root present")
        .expect("delivered");

        assert!(
            host.get(&root_id).unwrap().session().is_closed(),
            "the root self-closed but lingers registered until reap"
        );
        host.reap_closed_and_notify().await;
        assert!(
            !host.contains(&root_id),
            "the self-closed root is reaped (no parent to notify, clean removal)"
        );
        assert!(
            host.is_empty(),
            "no stray sessions remain after reaping the root"
        );
    }

    /// §6 supervision test reducer: self-completes ONCE on a `task/go` inbound (Success), and is INERT on any
    /// other inbound (returns `none`, does NOT re-close). Used as the closing PARENT in the two-self-close reap
    /// test: unlike `SelfClosingChildReducer` (which closes on ANY inbound), this stays inert on an injected
    /// `child-completed`, so if the reap ever wrongly delivered one to this closed parent the Inbound would
    /// land as the terminal tip (past `Closed`) — exactly the corruption the test must catch.
    struct CloseOnGoThenInertReducer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for CloseOnGoThenInertReducer {
        async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            if let EventBody::Inbound { content_type, .. } = &event.body {
                if content_type.matches_family("task/go") {
                    return FoldOutput::close(cdz_kernel::event::CloseOutcome::Success(
                        Payload::Inline(b"p-done".to_vec().into()),
                    ));
                }
            }
            FoldOutput::none()
        }
    }

    #[tokio::test]
    async fn reap_does_not_notify_a_parent_that_is_itself_closing_in_the_same_batch() {
        // §6 supervision — reviewer c267d8431 regression: a parent P and its child C both self-close in the
        // SAME reap batch. Since the kernel's `deliver` guards `is_terminated` but NOT `is_closed`, delivering
        // C's `child-completed` onto the still-registered-but-closed P would FOLD it PAST P's terminal `Closed`
        // event — corrupting the durable-log terminal-tip invariant and making P un-reapable on recovery. The
        // fix skips the notify to a parent that is itself in the closing set. Here we drive both self-closes,
        // reap, and assert via P's DURABLE log that its tip stays `Closed` (no child-completed appended past
        // it), both are reaped, and P's OWN completion still reached the grandparent G.
        use cdz_kernel::log_store::LogStore;

        let dir = crate::testutil::unique_temp_dir("reap-both-closed");
        let path = dir.join("parent-durable.log");

        let mut host = AgentHost::new();
        // Grandparent G — folds child-completed (records the completed child's hash + outcome).
        let g = HostedSession::genesis(
            Hash::of(b"grandparent-v1"),
            Box::new(ChildCompletedFoldingReducer),
            Box::new(Authorizer::deny_all()),
            CompositeExecutor::new(),
        );
        let g_hash = g.genesis_hash();
        let g_id = SessionId::new(Hash::of(b"orchestrator-g")); // vanity id (resolve-by-genesis-hash)
        host.spawn(g_id, g);

        // Parent P — a child of G, self-closes on task/go then INERT, with a DURABLE log so we can inspect its
        // persisted tail after it's reaped out of the registry.
        let p = HostedSession::genesis_spawned(
            Hash::of(b"parent-v1"),
            g_hash,
            Box::new(CloseOnGoThenInertReducer),
            Box::new(Authorizer::deny_all()),
            CompositeExecutor::new(),
        );
        let p_hash = p.genesis_hash();
        let p = p.with_sink(Box::new(LogStore::open(&path).expect("open parent log")));
        let p_id = SessionId::new(p_hash);
        host.spawn(p_id, p);

        // Child C — a child of P, self-closes on any inbound.
        let c = HostedSession::genesis_spawned(
            Hash::of(b"child-v1"),
            p_hash,
            Box::new(SelfClosingChildReducer),
            Box::new(Authorizer::deny_all()),
            CompositeExecutor::new(),
        );
        let c_id = SessionId::new(c.genesis_hash());
        host.spawn(c_id, c);

        let go = || EventBody::Inbound {
            content_type: cdz_kernel::event::ContentType {
                family: "task/go".into(),
                version: 1,
            },
            payload: Payload::Inline(b"work".to_vec().into()),
        };
        // Close C then P (both now closed + still registered, lingering until the reap).
        host.deliver(&c_id, go(), None)
            .await
            .expect("child present")
            .expect("child delivered");
        host.deliver(&p_id, go(), None)
            .await
            .expect("parent present")
            .expect("parent delivered");
        assert!(host.get(&c_id).unwrap().session().is_closed());
        assert!(host.get(&p_id).unwrap().session().is_closed());

        host.reap_closed_and_notify().await;

        // Both reaped from the registry.
        assert!(!host.contains(&c_id), "the self-closed child is reaped");
        assert!(!host.contains(&p_id), "the self-closed parent is reaped");

        // THE INVARIANT: P's DURABLE log tail is still `Closed` — no `lifecycle/child-completed` Inbound was
        // folded past it (which would happen if the reap notified the closing parent). Recovering the file is
        // exactly what a boot-recovery reads, so this pins the recovery-reap contract the reviewer flagged.
        let recovered = LogStore::recover(&path).expect("recover parent log");
        assert!(
            matches!(
                recovered.events.last().map(|e| &e.body),
                Some(EventBody::Closed { .. })
            ),
            "parent's durable tip stays Closed (no child-completed appended past the terminal event); got {:?}",
            recovered.events.last().map(|e| &e.body)
        );
        assert!(
            !recovered.events.iter().any(|e| matches!(
                &e.body,
                EventBody::Inbound { content_type, .. }
                    if content_type.matches_family("lifecycle/child-completed")
            )),
            "no child-completed Inbound was ever delivered to the closing parent"
        );

        // P's OWN completion still propagated UP to the grandparent G (P is not in the closed set from G's
        // perspective as a notify target — G is open — so the fix drops only the child->closing-parent hop).
        let g_kv = host.get(&g_id).unwrap();
        let g_kv = g_kv.session().kv();
        assert_eq!(
            g_kv.get(b"completed-child").map(|v| v.to_vec()),
            Some(p_hash.to_hex().into_bytes()),
            "the grandparent folded child-completed carrying P's hash (P's completion reached G)"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn session_id_by_genesis_hash_resolves_a_vanity_id_by_content_not_hex() {
        // #2484 c-a happy path: the resolver finds a session under an OPAQUE (vanity) id by matching its
        // genesis_hash, not by hex-ing the hash into a SessionId. This is the single-match (contract-normal)
        // case — exactly one session per genesis hash under the fresh-nonce contract.
        let mut host = AgentHost::new();
        let s = now_host();
        let genesis = s.genesis_hash();
        let vanity = SessionId::new(Hash::of(b"concierge"));
        assert_ne!(
            vanity.to_hex(),
            genesis.to_hex(),
            "registered under a vanity id, not its genesis-hash hex"
        );
        host.spawn(vanity, s);
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
        host.spawn(SessionId::new(Hash::of(b"aaa")), a);
        host.spawn(SessionId::new(Hash::of(b"bbb")), b);
        // Two registered sessions share a genesis hash → the resolver trips the uniqueness debug_assert.
        let _ = host.session_id_by_genesis_hash(&genesis);
    }

    #[tokio::test]
    async fn terminating_a_root_session_emits_no_child_exited_no_bounce_i7() {
        // §lifecycle I7 edge: a ROOT session (parent() == None) has no supervisor to notify — terminating it
        // is a clean no-op on the emit path (no panic, no bounce). Proven by terminating a plain root session.
        let mut host = AgentHost::new();
        let root = SessionId::new(Hash::of(b"root"));
        host.spawn(root, now_host());
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
        let parent = SessionId::new(Hash::of(b"gone-parent"));
        host.spawn(parent, now_host());
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
        let parent = SessionId::new(Hash::of(b"concierge")); // vanity id (also exercises the c1 genesis-hash lookup)
        host.spawn(parent, supervisor);

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
        let child_hash = child_id.hash();

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
            .terminate(
                &SessionId::new(Hash::of(b"ghost")),
                Hash::of(b"ctl"),
                "x".into(),
            )
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
        let parent = SessionId::new(Hash::of(b"parent"));
        host.spawn(parent, now_host());

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
            child_id.to_hex(),
            host.get(&child_id).unwrap().genesis_hash().to_hex(),
            "child SessionId = hex(its genesis_hash)"
        );
        // The parent's log carries exactly one Spawned edge, whose child_hash is the child's genesis hash.
        let edges = host.get(&parent).unwrap().spawned_children();
        assert_eq!(edges.len(), 1, "parent recorded one spawn edge");
        assert_eq!(
            edges[0].to_hex(),
            child_id.to_hex(),
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
        let parent = SessionId::new(Hash::of(b"parent"));
        host.spawn(parent, now_host());
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
            child_id.to_hex(),
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
                &SessionId::new(Hash::of(b"ghost-parent")),
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
        let parent = SessionId::new(Hash::of(b"dead-parent"));
        // Terminate the parent HostedSession BEFORE registering it (installs the Terminated tail), then
        // spawn it into the registry so spawn_child finds a registered-but-terminated parent.
        let mut parent_session = now_host();
        parent_session
            .terminate(Hash::of(b"ctl"), "kill".into())
            .await
            .expect("parent terminates");
        host.spawn(parent, parent_session);
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
        let root = SessionId::new(Hash::of(b"root"));
        host.spawn(root, now_host());
        let child = spawn_kid(&mut host, &root, b"child-1").await;
        let child2 = spawn_kid(&mut host, &root, b"child-2").await;
        let grandchild = spawn_kid(&mut host, &child, b"grandchild-1").await;

        let mut want = vec![
            child.to_hex().to_string(),
            child2.to_hex().to_string(),
            grandchild.to_hex().to_string(),
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
            vec![grandchild.to_hex().to_string()],
            "an intermediate session's descendant set is only ITS subtree"
        );
    }

    #[test]
    fn descendant_set_of_an_absent_or_childless_controller_is_empty() {
        let mut host = AgentHost::new();
        host.spawn(SessionId::new(Hash::of(b"lonely")), now_host());
        // A registered session with no spawns → empty; admits no target (denies all lifecycle control).
        assert_eq!(
            oneof_set(&host.descendant_set_of(&SessionId::new(Hash::of(b"lonely")))),
            Vec::<String>::new()
        );
        // An absent controller → empty (no tree to walk), fail-closed.
        assert_eq!(
            oneof_set(&host.descendant_set_of(&SessionId::new(Hash::of(b"ghost")))),
            Vec::<String>::new()
        );
    }

    #[test]
    fn suspend_resume_flips_the_scheduler_bit_without_touching_the_log() {
        // §lifecycle I4 mechanism: suspend/resume flip the host-scheduler bit (NOT a log mutation) +
        // idempotent; a suspended session is NOT terminated (orthogonal). AgentHost by-id + HostedSession
        // direct both work; absent id = false.
        let mut host = AgentHost::new();
        let id = SessionId::new(Hash::of(b"worker"));
        host.spawn(id, now_host());
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
        assert!(!host.suspend(&SessionId::new(Hash::of(b"ghost"))));
        assert!(!host.resume(&SessionId::new(Hash::of(b"ghost"))));
        assert!(!host.is_suspended(&SessionId::new(Hash::of(b"ghost"))));
    }

    #[tokio::test]
    async fn two_sessions_run_independently() {
        let mut host = AgentHost::new();
        host.spawn(SessionId::new(Hash::of(b"a")), now_host());
        host.spawn(SessionId::new(Hash::of(b"b")), now_host());
        // Drive only "a".
        host.deliver(&SessionId::new(Hash::of(b"a")), inbound_go(), None)
            .await;
        assert_eq!(
            host.get(&SessionId::new(Hash::of(b"a")))
                .unwrap()
                .session()
                .kv()
                .get(b"status")
                .as_deref(),
            Some(&b"ran"[..])
        );
        // "b" untouched — independent state.
        assert_eq!(
            host.get(&SessionId::new(Hash::of(b"b")))
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
        let id = SessionId::new(Hash::of(b"timed"));
        host.spawn(id, timer_host(1000));
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
        assert_eq!(
            hosted.session().kv().get(b"woke").as_deref(),
            Some(&b"1"[..])
        );
        assert_eq!(hosted.open_effects(), 0);
    }

    #[tokio::test]
    async fn host_fire_due_timers_sweeps_all_sessions_and_sums_fired() {
        // The all-session scheduler sweep: fire_due_timers(now) fires EVERY session's due timers and
        // returns the total count. Two sessions with different deadlines → a tick between them fires only
        // the earlier one; a later tick fires the other. A session with no timer contributes 0 (not woken).
        let mut host = AgentHost::new();
        host.spawn(SessionId::new(Hash::of(b"early")), timer_host(1000));
        host.spawn(SessionId::new(Hash::of(b"late")), timer_host(5000));
        host.spawn(SessionId::new(Hash::of(b"no-timer")), now_host()); // arms no timer
        host.deliver(&SessionId::new(Hash::of(b"early")), inbound_go(), None)
            .await;
        host.deliver(&SessionId::new(Hash::of(b"late")), inbound_go(), None)
            .await;
        // no-timer session gets no inbound → no armed timer.

        // Tick at 1000: only "early" is due → 1 fired total.
        assert_eq!(host.fire_due_timers(1000).await, 1);
        assert_eq!(
            host.get(&SessionId::new(Hash::of(b"early")))
                .unwrap()
                .session()
                .kv()
                .get(b"woke")
                .as_deref(),
            Some(&b"1"[..])
        );
        assert_eq!(
            host.get(&SessionId::new(Hash::of(b"late")))
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
            host.get(&SessionId::new(Hash::of(b"late")))
                .unwrap()
                .session()
                .kv()
                .get(b"woke")
                .as_deref(),
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

        host.spawn(SessionId::new(Hash::of(b"late")), timer_host(5000));
        host.spawn(SessionId::new(Hash::of(b"early")), timer_host(1000));
        host.spawn(SessionId::new(Hash::of(b"no-timer")), now_host()); // arms no timer

        // Before any inbound, no session has armed its timer yet → still None.
        assert_eq!(host.next_timer_deadline_across_sessions(), None);

        // Arm both timers (the no-timer session gets no inbound, so it contributes nothing).
        host.deliver(&SessionId::new(Hash::of(b"late")), inbound_go(), None)
            .await;
        host.deliver(&SessionId::new(Hash::of(b"early")), inbound_go(), None)
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
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
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
        let id = SessionId::new(Hash::of(b"worker"));
        host.spawn(
            id,
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
        assert_eq!(
            hosted.session().kv().get(b"phase").as_deref(),
            Some(&b"working"[..])
        );
        // The live session's tip seq is the non-interference witness (log-decouple I5: no resident Vec to
        // length-count; the tip seq advances per appended event, so an unchanged seq == the log didn't grow).
        let live_tip_seq = hosted.session().snapshot().seq;

        // Fork-for-query it: caller supplies the same (native Reducer) reducer + a model-only authz
        // (deny_all here — the summarize fold takes no world-effects; the control/summary effect is
        // authz-exempt) + an executor. Returns the summary carried on the control-plane channel.
        let mut exec = CompositeExecutor::new();
        let summary = hosted
            .fork_for_query(&mut ReportingAgent, &Authorizer::deny_all(), &mut exec)
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
        assert_eq!(
            hosted.session().kv().get(b"phase").as_deref(),
            Some(&b"working"[..])
        );
        assert_eq!(hosted.session().snapshot().seq, live_tip_seq);
    }

    /// A report-aware agent that emits control effects on a `report` — but emits `control/capabilities`
    /// FIRST and `control/summary` SECOND, plus a non-summary payload on the capabilities one. Proves the
    /// fork reads the summary by FILTERING on family, not by taking the first control effect.
    struct MultiControlAgent;
    #[async_trait::async_trait(?Send)]
    impl Reducer for MultiControlAgent {
        async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
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
        let id = SessionId::new(Hash::of(b"multi"));
        host.spawn(
            id,
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
            .fork_for_query(&mut MultiControlAgent, &Authorizer::deny_all(), &mut exec)
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
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
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
        let id = SessionId::new(Hash::of(b"silent"));
        host.spawn(
            id,
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
            .fork_for_query(&mut NoSummaryAgent, &Authorizer::deny_all(), &mut exec)
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
        async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
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
        let id = SessionId::new(Hash::of(b"blob-then-inline"));
        host.spawn(
            id,
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
                &mut BlobThenInlineSummaryAgent,
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
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
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
            hosted.session().kv().get(b"capabilities").as_deref(),
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

    #[tokio::test]
    async fn a_policy_swap_pushes_a_capabilities_change_the_agent_observes() {
        // §20b policy-swap OBSERVABILITY as a UNIT test (operator directive: replace the 60s component-linking
        // policy_swap_e2e with fast units — an in-process Authorizer swap exercises the SAME swap→push→fold
        // mechanism as a live Cedar-guest reload, minus the wasm component link). The E2E's only genuinely
        // end-to-end part is `ComponentAuthorizer::from_policy_bytes` lifting a real Cedar guest; that lift is
        // covered by cedar_authz_e2e. What THIS proves — the mechanism the host owns — is: a session that
        // STARTS deny-all, after a policy swap + push_capabilities_changed, folds a capabilities-changed
        // manifest to the agent that MOVED and equals EXACTLY the new policy's projected surface.
        let served =
            || CompositeExecutor::new().with_effect(effect_ct::NOW, Box::new(ClockExecutor::new()));
        let mut hosted = HostedSession::genesis(
            Hash::of(b"policy-swap-unit-v1"),
            Box::new(CapabilityAwareAgent),
            Box::new(Authorizer::deny_all()),
            served(),
        );
        hosted.seed_capabilities().await;
        let manifest_under_deny_all = hosted
            .session()
            .kv()
            .get(b"capabilities")
            .expect("seeded baseline manifest")
            .to_vec();

        // LIVE SWAP to a policy that PERMITS Now (stands in for the Cedar guest's broad grant), then push —
        // the same set_authorizer + push_capabilities_changed path reload_policy_from_component_bytes runs
        // after the wasm lift.
        let new_policy = Authorizer::new(vec![Capability {
            kind: EffectKind::Now,
            predicate: ResourcePredicate::Any,
        }]);
        hosted.set_authorizer(Box::new(Authorizer::new(vec![Capability {
            kind: EffectKind::Now,
            predicate: ResourcePredicate::Any,
        }])));
        let pushed = hosted.push_capabilities_changed().await;
        assert!(
            pushed.is_empty(),
            "the capabilities-changed push is answered inline"
        );

        let manifest_after_swap = hosted
            .session()
            .kv()
            .get(b"capabilities")
            .expect("a capabilities-changed folded after the swap")
            .to_vec();
        // The manifest MOVED (deny-all → permit-Now is a different grant surface) AND equals exactly the
        // manifest the kernel projects over this session's served surface against the NEW policy — proving the
        // swap installed THAT policy + is observable to the agent, not merely "changed".
        assert_ne!(
            manifest_after_swap, manifest_under_deny_all,
            "the policy swap changed the session's capability manifest (observable to the agent)"
        );
        assert_eq!(
            manifest_after_swap,
            expected_manifest(&served(), &new_policy).await,
            "the pushed manifest is exactly the one the newly-swapped policy projects (the swap installed \
             THAT policy)"
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
        ) -> Result<crate::HttpResponse, cdz_kernel::event::EffectOutcome> {
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
            hosted.session().kv().get(b"capabilities").as_deref(),
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
            hosted.session().kv().get(b"capabilities").as_deref(),
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
        // The tip seq before the push (log-decouple I5: no resident Vec to length-count; the tip seq advances
        // per appended event, so an unchanged seq == nothing was appended).
        let tip_seq_before = hosted.session().snapshot().seq;

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
            hosted.session().snapshot().seq,
            tip_seq_before,
            "no capability change → nothing appended to the log (the coalescing/gate)"
        );
    }

    // ── §4c AWS-backends I4a: canonical name-store snapshot durability ──────────────────────────────────

    /// Permits any `store/*` effect — the authz gate a store effect passes through (mirrors the kernel test
    /// `AllowStore`). A real deployment uses a name-prefix-scoped grant; here we prove the host's snapshot
    /// mutation-hook, so a blanket store-permitting authorizer isolates that from the grant-shape work.
    struct AllowStore;
    #[async_trait::async_trait(?Send)]
    impl Authorize for AllowStore {
        async fn authorize(&self, req: &cdz_kernel::effect::EffectRequest) -> Result<(), String> {
            if effect_ct::is_store_family(&req.content_type.family) {
                Ok(())
            } else {
                Err("only store/* permitted".into())
            }
        }
    }

    /// A reducer that on inbound emits a single `store/set COMPILER_LATEST → <hash>` — enough to mutate the
    /// session's name store so the host folds it into canonical + snapshots.
    struct StoreSetAgent {
        value: Hash,
    }
    #[async_trait::async_trait(?Send)]
    impl Reducer for StoreSetAgent {
        async fn fold(&mut self, event: &cdz_kernel::event::Event, _kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    let payload = cdz_kernel::event_ast::encode_name_set(
                        cdz_kernel::name_store::NameStore::COMPILER_LATEST,
                        &self.value,
                    );
                    FoldOutput::with(vec![EffectRequest::new_with_family(
                        effect_ct::STORE_SET,
                        cdz_kernel::name_store::NameStore::COMPILER_LATEST,
                        Some(Payload::Inline(payload.into())),
                        Timeliness::Interactive,
                    )])
                }
                _ => FoldOutput::none(),
            }
        }
    }

    fn store_set_host(value: Hash) -> HostedSession {
        HostedSession::genesis(
            Hash::of(b"store-set-agent-v1"),
            Box::new(StoreSetAgent { value }),
            Box::new(AllowStore),
            CompositeExecutor::new(),
        )
    }

    #[tokio::test]
    async fn deliver_snapshots_the_canonical_store_after_a_session_writes_a_name() {
        use crate::name_snapshot::MemNameStoreSnapshot;
        use cdz_kernel::name_store::NameStore;

        // A canonical-backed host WITH a snapshot store: a deliver that writes a name folds into the canonical
        // store AND fires the mutation-hook without erroring the turn. (The DURABLE round-trip — that the
        // saved bytes restore the pointer into a fresh host — is pinned by
        // `snapshot_persists_across_a_restart_via_a_shared_backend`; the boxed snapshot store is held
        // privately, so this test observes the fold + that the hook ran clean.)
        let value = Hash::of(b"compiler-wasm-v1");
        let mut host = AgentHost::with_canonical_store(NameStore::new())
            .with_name_snapshot_store(Box::new(MemNameStoreSnapshot::new()));
        let id = host.spawn(SessionId::new(Hash::of(b"writer")), store_set_host(value));

        let outcome = host.deliver(&id, inbound_go(), None).await;
        assert!(
            matches!(outcome, Some(Ok(()))),
            "the write turn runs (hook fired clean)"
        );

        // The canonical store now resolves the written pointer.
        assert_eq!(
            host.canonical_store()
                .unwrap()
                .borrow()
                .resolve(NameStore::COMPILER_LATEST)
                .unwrap(),
            value,
            "the session's store/set folded into the canonical store"
        );
        assert_eq!(
            host.canonical_store()
                .unwrap()
                .borrow()
                .to_set_entries()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn snapshot_persists_across_a_restart_via_a_shared_backend() {
        // End-to-end durability: a first host writes + snapshots to a MemNameStoreSnapshot; a SECOND host
        // built with `with_canonical_store_restored` over the SAME backend boots with the pointer already
        // present (survives a "restart"). MemNameStoreSnapshot is process-local, so to share it across the two
        // hosts we drive the save/restore through the same bytes: capture the snapshot the first host saved by
        // reading the canonical store's own snapshot_bytes (byte-identical to what save wrote), feed it to a
        // pre-seeded backend, and restore the second host from it.
        use crate::name_snapshot::{MemNameStoreSnapshot, NameStoreSnapshotStore};
        use cdz_kernel::name_store::NameStore;

        let value = Hash::of(b"compiler-wasm-v7");
        let mut host1 = AgentHost::with_canonical_store(NameStore::new())
            .with_name_snapshot_store(Box::new(MemNameStoreSnapshot::new()));
        let id = host1.spawn(SessionId::new(Hash::of(b"writer")), store_set_host(value));
        host1.deliver(&id, inbound_go(), None).await;

        // The bytes the host durably saved == the canonical store's snapshot_bytes (the save argument).
        let saved = host1.canonical_store().unwrap().borrow().snapshot_bytes();
        assert!(
            !saved.is_empty(),
            "a non-empty snapshot was produced from the write"
        );

        // Simulate the durable backend surviving a restart: a fresh backend pre-seeded with the saved bytes.
        let mut backend = MemNameStoreSnapshot::new();
        backend.save(&saved).await.unwrap();

        // A NEW host restores its canonical store from that backend on boot.
        let host2 = AgentHost::with_canonical_store_restored(Box::new(backend)).await;
        assert_eq!(
            host2.canonical_store().unwrap().borrow().resolve(NameStore::COMPILER_LATEST).unwrap(),
            value,
            "the restored host boots with the previously-published pointer (durable across a restart)"
        );
    }

    #[tokio::test]
    async fn with_canonical_store_restored_starts_empty_when_nothing_saved() {
        use crate::name_snapshot::MemNameStoreSnapshot;

        // No prior snapshot (a fresh deployment) → the restored host boots with an empty canonical store, not
        // a panic. It IS canonical-backed (so subsequent writes snapshot going forward).
        let host =
            AgentHost::with_canonical_store_restored(Box::new(MemNameStoreSnapshot::new())).await;
        assert!(
            host.canonical_store().is_some(),
            "restore leaves a canonical-backed host"
        );
        assert!(
            host.canonical_store()
                .unwrap()
                .borrow()
                .to_set_entries()
                .is_empty(),
            "nothing saved yet → an empty canonical store"
        );
    }

    #[tokio::test]
    async fn with_canonical_store_restored_starts_empty_on_a_corrupt_snapshot() {
        use crate::name_snapshot::{MemNameStoreSnapshot, NameStoreSnapshotStore};

        // A corrupt/garbled snapshot must NOT wedge the daemon at boot: it starts EMPTY (logged), never panics.
        let mut backend = MemNameStoreSnapshot::new();
        backend.save(&[1u8, 2, 3]).await.unwrap(); // a short/garbage blob → MalformedSnapshot on restore
        let host = AgentHost::with_canonical_store_restored(Box::new(backend)).await;
        assert!(host.canonical_store().is_some());
        assert!(
            host.canonical_store()
                .unwrap()
                .borrow()
                .to_set_entries()
                .is_empty(),
            "a corrupt snapshot falls back to an empty store (best-effort durability, no panic)"
        );
    }

    #[tokio::test]
    async fn a_share_less_host_with_a_snapshot_store_is_inert() {
        use crate::name_snapshot::MemNameStoreSnapshot;

        // A snapshot store on a SHARE-LESS host does nothing (no canonical to mutate → nothing saved), and a
        // turn runs unchanged. Proves the "only when canonical-backed AND snapshot-store-set" gate.
        let mut host =
            AgentHost::new().with_name_snapshot_store(Box::new(MemNameStoreSnapshot::new()));
        assert!(host.canonical_store().is_none());
        let id = host.spawn(
            SessionId::new(Hash::of(b"writer")),
            store_set_host(Hash::of(b"v")),
        );
        let outcome = host.deliver(&id, inbound_go(), None).await;
        assert!(
            matches!(outcome, Some(Ok(()))),
            "a share-less turn runs unchanged"
        );
    }

    /// A signature-query agent (§signature-query part-1): on inbound, it emits a `control/signature` effect
    /// naming a target component (by a hex hash in the effect target). When the host folds the reflected
    /// descriptor back as the EffectResult, it records the outcome into KV — `sig-ok` + the descriptor bytes
    /// on Ok, or `sig-err` on an Err (so a test can assert the reducer RESUMED with whichever arm).
    struct SignatureQueryAgent {
        target_hex: String,
    }
    #[async_trait::async_trait(?Send)]
    impl Reducer for SignatureQueryAgent {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    FoldOutput::with(vec![EffectRequest::new_with_family(
                        effect_ct::SIGNATURE,
                        self.target_hex.clone(),
                        None,
                        Timeliness::Interactive,
                    )])
                }
                EventBody::EffectResult { result, .. } => {
                    match result {
                        EffectOutcome::Ok(Some(Payload::Inline(bytes))) => {
                            kv.put(b"sig-ok".to_vec(), bytes.to_vec());
                        }
                        EffectOutcome::Err { .. } => {
                            kv.put(b"sig-err".to_vec(), b"1".to_vec());
                        }
                        _ => {}
                    }
                    FoldOutput::none()
                }
                _ => FoldOutput::none(),
            }
        }
    }

    fn signature_query_host(target_hex: &str) -> HostedSession {
        // control/* is authz-EXEMPT (control-plane), so a deny-all authorizer still lets the signature query
        // through — proving the introspection needs no capability grant.
        HostedSession::genesis(
            Hash::of(b"sigquery-agent-v1"),
            Box::new(SignatureQueryAgent {
                target_hex: target_hex.to_string(),
            }),
            Box::new(Authorizer::deny_all()),
            CompositeExecutor::new(),
        )
    }

    #[tokio::test]
    async fn signature_query_surfaces_a_control_effect_the_host_can_settle() {
        // deliver_surfacing_controls must SURFACE the reducer's control/signature effect (the common deliver
        // drops it). Assert the surfaced set carries exactly that family, with an id (the settle key) + the
        // target the reducer named.
        let target = Hash::of(b"some-target-component");
        let mut host = signature_query_host(&target.to_hex());
        let controls = host
            .deliver_surfacing_controls(inbound_go(), None)
            .await
            .expect("deliver ok");
        let sig: Vec<_> = controls
            .iter()
            .filter(|ce| ce.request.content_type.matches_family(effect_ct::SIGNATURE))
            .collect();
        assert_eq!(sig.len(), 1, "the control/signature effect was surfaced");
        assert_eq!(
            sig[0].request.target_str().expect("a hex target is UTF-8"),
            target.to_hex(),
            "the surfaced effect carries the reducer-named target"
        );
    }

    #[tokio::test]
    async fn settle_signature_query_absent_target_folds_the_err_arm_and_resumes() {
        // The fold-back seam: a target NOT in the blob store (None) settles an Err, and the reducer RESUMES on
        // its EffectResult-Err arm (records sig-err) rather than hanging on the open effect. Hermetic — no
        // wasm component needed (the None path never reflects).
        let target = Hash::of(b"missing-target");
        let mut host = signature_query_host(&target.to_hex());
        let controls = host
            .deliver_surfacing_controls(inbound_go(), None)
            .await
            .expect("deliver ok");
        let ce = controls
            .into_iter()
            .find(|ce| ce.request.content_type.matches_family(effect_ct::SIGNATURE))
            .expect("the signature effect surfaced");
        let settled = host.settle_signature_query(&ce, None).await;
        assert!(settled, "an open control/signature id settles");
        assert_eq!(
            host.session().kv().get(b"sig-err").as_deref(),
            Some(&b"1"[..]),
            "the reducer resumed on the Err arm (absent target → settled Err, not hung)"
        );
    }

    #[tokio::test]
    async fn settle_signature_query_non_component_bytes_folds_the_err_arm() {
        // Bytes that aren't a valid component → component_signature_from_bytes_owned Errs → settle Err → the
        // reducer resumes on its Err arm (never a panic on garbage target bytes).
        let target = Hash::of(b"bogus-target");
        let mut host = signature_query_host(&target.to_hex());
        let controls = host
            .deliver_surfacing_controls(inbound_go(), None)
            .await
            .expect("deliver ok");
        let ce = controls
            .into_iter()
            .find(|ce| ce.request.content_type.matches_family(effect_ct::SIGNATURE))
            .expect("the signature effect surfaced");
        let settled = host
            .settle_signature_query(&ce, Some(b"not a wasm component"))
            .await;
        assert!(settled, "the id settles even on a reflect failure");
        assert_eq!(
            host.session().kv().get(b"sig-err").as_deref(),
            Some(&b"1"[..]),
            "un-reflectable target bytes settle an Err the reducer folds, not a panic"
        );
    }

    #[tokio::test]
    async fn deliver_answering_signatures_surfaces_and_settles_through_the_agent_host() {
        // The slice-2b loop path end-to-end at the AgentHost level: deliver_answering_signatures surfaces the
        // session's control/signature effect, resolves the target via the factory (None here → absent), and
        // settles the Err arm so the reducer RESUMES (records sig-err) — proving the surface+resolve+settle
        // wiring the async loop uses, hermetically (the absent path needs no wasm component).
        let target = Hash::of(b"target-not-in-any-store");
        let mut host = AgentHost::new();
        let id = host.spawn(
            SessionId::new(Hash::of(b"sq")),
            signature_query_host(&target.to_hex()),
        );
        // No factory → the target resolves to None → settle the Err arm.
        let outcome = host
            .deliver_answering_signatures(&id, inbound_go(), None, None)
            .await;
        assert!(matches!(outcome, Some(Ok(()))), "the turn ran");
        assert_eq!(
            host.sessions.get(&id).unwrap().session().kv().get(b"sig-err").as_deref(),
            Some(&b"1"[..]),
            "the loop surfaced the signature query + settled the Err arm (no factory → absent target), \
             and the reducer resumed"
        );
    }

    // ---- §4c store/* over a hosted session (converted from the deleted name_store_e2e integration test,
    // operator no-integration-tests mandate — same coverage as in-crate units: no wasm, HostedSession +
    // Rust reducers + the real Authorizer). ----

    const STORE_NAME: &str = "system/compiler/latest";

    /// Publishes then reads back a well-known pointer: on inbound `store/set`s STORE_NAME → <hash>; on that
    /// settle `store/resolve`s it; on the resolve settle records the resolved hash's hex in KV.
    struct SetThenResolve;
    #[async_trait::async_trait(?Send)]
    impl Reducer for SetThenResolve {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
            use cdz_kernel::event_ast::{decode_name_set, encode_name_set};
            match &event.body {
                EventBody::Inbound { .. } => {
                    let payload = encode_name_set(STORE_NAME, &Hash::of(b"compiler-wasm-v1"));
                    FoldOutput::with(vec![EffectRequest::new_with_family(
                        effect_ct::STORE_SET,
                        STORE_NAME,
                        Some(Payload::Inline(payload.into())),
                        Timeliness::Interactive,
                    )])
                }
                EventBody::EffectResult {
                    result: EffectOutcome::Ok(body),
                    ..
                } => match kv.get(b"phase") {
                    None => {
                        kv.put(b"phase".to_vec(), b"resolving".to_vec());
                        FoldOutput::with(vec![EffectRequest::new_with_family(
                            effect_ct::STORE_RESOLVE,
                            STORE_NAME,
                            None,
                            Timeliness::Interactive,
                        )])
                    }
                    Some(_) => {
                        if let Some(Payload::Inline(bytes)) = body {
                            if let Ok((_n, h)) = decode_name_set(bytes) {
                                kv.put(b"resolved".to_vec(), h.to_hex().into_bytes());
                            }
                        }
                        FoldOutput::none()
                    }
                },
                _ => FoldOutput::none(),
            }
        }
    }

    /// Only tries to WRITE — on inbound a single `store/set` (no resolve), so a test can assert the write is
    /// denied without a resolve muddying the picture.
    struct SetOnly;
    #[async_trait::async_trait(?Send)]
    impl Reducer for SetOnly {
        async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            use cdz_kernel::event_ast::encode_name_set;
            if matches!(event.body, EventBody::Inbound { .. }) {
                let payload = encode_name_set(STORE_NAME, &Hash::of(b"compiler-wasm-v1"));
                FoldOutput::with(vec![EffectRequest::new_with_family(
                    effect_ct::STORE_SET,
                    STORE_NAME,
                    Some(Payload::Inline(payload.into())),
                    Timeliness::Interactive,
                )])
            } else {
                FoldOutput::none()
            }
        }
    }

    fn store_inbound_go() -> EventBody {
        EventBody::Inbound {
            content_type: ContentType {
                family: "message".into(),
                version: 1,
            },
            payload: Payload::Inline(b"go".to_vec().into()),
        }
    }

    /// Grant both store actions on the `system/` prefix (the §4c write+read authority a publisher gets).
    fn set_and_resolve_system() -> Authorizer {
        Authorizer::new(vec![]).with_family_grants(vec![
            Capability::for_family(
                effect_ct::STORE_SET,
                ResourcePredicate::Prefix("system/".into()),
            ),
            Capability::for_family(
                effect_ct::STORE_RESOLVE,
                ResourcePredicate::Prefix("system/".into()),
            ),
        ])
    }

    /// Grant ONLY resolve on `system/` — a read-only consumer; a `store/set` is denied (allow-read-deny-write).
    fn resolve_only_system() -> Authorizer {
        Authorizer::new(vec![]).with_family_grants(vec![Capability::for_family(
            effect_ct::STORE_RESOLVE,
            ResourcePredicate::Prefix("system/".into()),
        )])
    }

    #[tokio::test]
    async fn a_hosted_agents_store_set_then_resolve_round_trips_through_its_attached_name_store() {
        let mut session = HostedSession::genesis(
            Hash::of(b"publisher-v1"),
            Box::new(SetThenResolve),
            Box::new(set_and_resolve_system()),
            CompositeExecutor::new(),
        )
        .with_name_store(cdz_kernel::name_store::NameStore::new());

        session.deliver(store_inbound_go(), None).await.unwrap();

        // The set applied and the resolve read the latest — through the host's attached store.
        assert_eq!(
            session.session().kv().get(b"resolved").as_deref(),
            Some(Hash::of(b"compiler-wasm-v1").to_hex().as_bytes()),
            "store/set → store/resolve round-tripped through the attached NameStore"
        );
        assert_eq!(
            session.open_effects(),
            0,
            "both store effects settled (store/* is not executor-routed)"
        );
    }

    #[tokio::test]
    async fn a_resolve_only_grant_denies_a_store_set_allow_read_deny_write() {
        let (sink, captured) = crate::testutil::log_capture::recording_sink();
        let mut session = HostedSession::genesis(
            Hash::of(b"consumer-v1"),
            Box::new(SetOnly),
            Box::new(resolve_only_system()),
            CompositeExecutor::new(),
        )
        .with_name_store(cdz_kernel::name_store::NameStore::new())
        .with_sink(sink);

        session.deliver(store_inbound_go(), None).await.unwrap();

        // Write authority is a SEPARATE grant: resolve-only → the store/set is denied at the gate (§4c
        // allow-read-deny-write), on the log (read from the recording sink, I5), nothing left open.
        assert!(
            captured
                .borrow()
                .iter()
                .any(|e| matches!(e.body, EventBody::AuthzDenied { .. })),
            "a store/set under a resolve-only grant is denied"
        );
        assert_eq!(session.open_effects(), 0);
    }

    #[tokio::test]
    async fn a_store_effect_with_no_name_store_attached_folds_an_error_not_a_panic() {
        // Plain genesis (no with_name_store) → no store bound. A store/* effect must fold an observable Err
        // (§9d/§17), never panic. The grant permits it, so this exercises the missing-store path, not authz.
        let (sink, captured) = crate::testutil::log_capture::recording_sink();
        let mut session = HostedSession::genesis(
            Hash::of(b"no-store-v1"),
            Box::new(SetOnly),
            Box::new(set_and_resolve_system()),
            CompositeExecutor::new(),
        )
        .with_sink(sink);

        session.deliver(store_inbound_go(), None).await.unwrap();
        assert_eq!(
            session.open_effects(),
            0,
            "the store effect settled (as an Err) — no hang, no panic"
        );
        assert!(
            captured.borrow().iter().any(|e| matches!(
                &e.body,
                EventBody::EffectResult {
                    result: EffectOutcome::Err { .. },
                    ..
                }
            )),
            "a store/* effect with no attached store folds an observable Err"
        );
    }

    // ---- §4c v0.3 SHARED canonical store lifecycle (converted from the deleted shared_store_host_e2e
    // integration test, operator no-integration-tests mandate — hermetic: AgentHost + HostedSession + Rust
    // reducers, no wasm/network). Exercises the shared-canonical refactor (with_canonical_store + spawn-replay
    // + merge-back) that this file owns. ----

    const SHARED_POINTER: &str = cdz_kernel::name_store::NameStore::COMPILER_LATEST;

    /// PUBLISHER: on inbound, `store/set`s SHARED_POINTER → the hash it carries.
    struct SharedPublisher {
        hash: Hash,
    }
    #[async_trait::async_trait(?Send)]
    impl Reducer for SharedPublisher {
        async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            use cdz_kernel::event_ast::encode_name_set;
            if matches!(event.body, EventBody::Inbound { .. }) {
                FoldOutput::with(vec![EffectRequest::new_with_family(
                    effect_ct::STORE_SET,
                    SHARED_POINTER,
                    Some(Payload::Inline(
                        encode_name_set(SHARED_POINTER, &self.hash).into(),
                    )),
                    Timeliness::Interactive,
                )])
            } else {
                FoldOutput::none()
            }
        }
    }

    /// CONSUMER: on inbound, `store/resolve`s SHARED_POINTER; records the resolved hash's hex in KV.
    struct SharedConsumer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for SharedConsumer {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
            use cdz_kernel::event_ast::decode_name_set;
            match &event.body {
                EventBody::Inbound { .. } => {
                    FoldOutput::with(vec![EffectRequest::new_with_family(
                        effect_ct::STORE_RESOLVE,
                        SHARED_POINTER,
                        None,
                        Timeliness::Interactive,
                    )])
                }
                EventBody::EffectResult {
                    result: EffectOutcome::Ok(Some(Payload::Inline(bytes))),
                    ..
                } => {
                    if let Ok((_n, h)) = decode_name_set(bytes) {
                        kv.put(b"resolved".to_vec(), h.to_hex().into_bytes());
                    }
                    FoldOutput::none()
                }
                _ => FoldOutput::none(),
            }
        }
    }

    fn share_set_system() -> Box<Authorizer> {
        Box::new(
            Authorizer::new(vec![]).with_family_grants(vec![Capability::for_family(
                effect_ct::STORE_SET,
                ResourcePredicate::Prefix("system/".into()),
            )]),
        )
    }
    fn share_resolve_system() -> Box<Authorizer> {
        Box::new(
            Authorizer::new(vec![]).with_family_grants(vec![Capability::for_family(
                effect_ct::STORE_RESOLVE,
                ResourcePredicate::Prefix("system/".into()),
            )]),
        )
    }

    #[tokio::test]
    async fn a_later_spawned_session_sees_what_an_earlier_session_published_via_the_canonical_store(
    ) {
        let published = Hash::of(b"compiler-wasm-v3");
        let mut host = AgentHost::with_canonical_store(cdz_kernel::name_store::NameStore::new());

        // PUBLISH: spawn a publisher (born with a replay of the empty canonical), deliver "go" → it sets the
        // pointer; on deliver return the host folds its write back into canonical (merge_appends_from).
        let pub_id = host.spawn(
            SessionId::new(Hash::of(b"publisher")),
            HostedSession::genesis(
                Hash::of(b"publisher-v1"),
                Box::new(SharedPublisher { hash: published }),
                share_set_system(),
                CompositeExecutor::new(),
            ),
        );
        host.deliver(&pub_id, inbound_go(), None)
            .await
            .expect("publisher session exists")
            .expect("the publish turn ran");

        // CONSUME: spawn a DIFFERENT session LATER — born with a replay of the NOW-updated canonical, so it
        // already carries the publisher's pointer, no explicit export/replay bridge.
        let con_id = host.spawn(
            SessionId::new(Hash::of(b"consumer")),
            HostedSession::genesis(
                Hash::of(b"consumer-v1"),
                Box::new(SharedConsumer),
                share_resolve_system(),
                CompositeExecutor::new(),
            ),
        );
        host.deliver(&con_id, inbound_go(), None)
            .await
            .expect("consumer session exists")
            .expect("the resolve turn ran");

        let resolved = host
            .get(&con_id)
            .expect("consumer registered")
            .session()
            .kv()
            .get(b"resolved")
            .expect("consumer recorded a resolved hash");
        assert_eq!(
            resolved,
            published.to_hex().as_bytes(),
            "a later-spawned consumer resolved COMPILER_LATEST to what the earlier publisher set — via the \
             canonical shared store, no explicit bridge"
        );
    }

    #[tokio::test]
    async fn a_share_less_host_leaves_sessions_store_less() {
        // A plain AgentHost::new() (no canonical) attaches NO store → a store/* effect folds an observable
        // Err (never a panic) — the opt-in boundary. Two share-less sessions never see each other's
        // (nonexistent) name space (no cross-session leak).
        let mut host = AgentHost::new();
        let id = host.spawn(
            SessionId::new(Hash::of(b"no-store")),
            HostedSession::genesis(
                Hash::of(b"no-store-v1"),
                Box::new(SharedConsumer),
                share_resolve_system(),
                CompositeExecutor::new(),
            ),
        );
        host.deliver(&id, inbound_go(), None)
            .await
            .expect("session exists")
            .expect("the turn ran (the store/* effect settled as an Err, no panic)");
        assert_eq!(
            host.get(&id).unwrap().open_effects(),
            0,
            "the resolve settled (as an Err — no store attached on a share-less host)"
        );
        assert!(
            host.get(&id)
                .unwrap()
                .session()
                .kv()
                .get(b"resolved")
                .is_none(),
            "nothing resolved — a share-less host attaches no store"
        );

        let id2 = host.spawn(
            SessionId::new(Hash::of(b"no-store-2")),
            HostedSession::genesis(
                Hash::of(b"no-store-2-v1"),
                Box::new(SharedConsumer),
                share_resolve_system(),
                CompositeExecutor::new(),
            ),
        );
        host.deliver(&id2, inbound_go(), None)
            .await
            .expect("session 2 exists")
            .expect("session 2's turn ran (store/* settled as an Err, no panic)");
        assert!(
            host.get(&id2)
                .unwrap()
                .session()
                .kv()
                .get(b"resolved")
                .is_none(),
            "a second share-less session also resolves nothing — no store handed down, no cross-session leak"
        );
    }

    // ---- §4c publish→consume through a hosted agent (converted from the deleted name_store_publish_consume_e2e
    // + name_store_two_agent_e2e integration tests, operator no-integration-tests mandate). ENV-GATED on
    // CDZ_LIVE_REDUCER_COMPONENT (a real lifted wasm reducer the nix build produces): unset → SKIP cleanly (a
    // plain cargo test has no such artifact), so the hermetic default gate is unaffected; set (the nix job) →
    // exercise the real publish→resolve→load→RUN arc. These prove a resolved pointer names a RUNNABLE artifact
    // — the one thing the in-session store round-trips above can't cover. ----
    use crate::test_support::reducer_component_bytes;
    use cdz_kernel::blob::{BlobStore, MemBlobStore};
    use cdz_kernel::event_ast::{decode_name_set, encode_name_set};
    use cdz_kernel::name_store::NameStore;
    use cdz_kernel::wasm_host::AsyncComponentReducer;
    use std::time::Duration;

    /// The well-known pointer both arcs use (the kernel-side source of truth).
    const PUBLISH_POINTER: &str = NameStore::COMPILER_LATEST;

    /// A hard ceiling on running a RESOLVED (real, externally-supplied) wasm artifact's fold turn.
    /// `HostedSession::deliver` drives the reducer→effect loop to quiescence with no step bound, so a
    /// misbehaving/looping live reducer could hang the suite — bounding it surfaces a runaway as a clear error.
    const FOLD_RUN_TIMEOUT: Duration = Duration::from_secs(30);

    fn publish_go() -> EventBody {
        EventBody::Inbound {
            content_type: ContentType {
                family: "message".into(),
                version: 1,
            },
            payload: Payload::Inline(b"go".to_vec().into()),
        }
    }

    /// A publisher/consumer reducer: on inbound `store/set`s the pointer → `artifact_hash`; when that settles
    /// `store/resolve`s it; when THAT settles records the resolved hash's hex in KV (so the test blob-gets +
    /// runs the artifact).
    struct PublishThenResolve {
        artifact_hash: Hash,
    }
    #[async_trait::async_trait(?Send)]
    impl Reducer for PublishThenResolve {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    let payload = encode_name_set(PUBLISH_POINTER, &self.artifact_hash);
                    FoldOutput::with(vec![EffectRequest::new_with_family(
                        effect_ct::STORE_SET,
                        PUBLISH_POINTER,
                        Some(Payload::Inline(payload.into())),
                        Timeliness::Interactive,
                    )])
                }
                EventBody::EffectResult {
                    result: EffectOutcome::Ok(body),
                    ..
                } => match kv.get(b"phase") {
                    None => {
                        kv.put(b"phase".to_vec(), b"resolving".to_vec());
                        FoldOutput::with(vec![EffectRequest::new_with_family(
                            effect_ct::STORE_RESOLVE,
                            PUBLISH_POINTER,
                            None,
                            Timeliness::Interactive,
                        )])
                    }
                    Some(_) => {
                        if let Some(Payload::Inline(bytes)) = body {
                            if let Ok((_n, h)) = decode_name_set(bytes) {
                                kv.put(b"resolved".to_vec(), h.to_hex().into_bytes());
                            }
                        }
                        FoldOutput::none()
                    }
                },
                _ => FoldOutput::none(),
            }
        }
    }

    /// A publisher that may set + resolve the well-known (`system/…`) pointer.
    fn publisher_authz() -> Authorizer {
        Authorizer::new(vec![]).with_family_grants(vec![
            Capability::for_family(
                effect_ct::STORE_SET,
                ResourcePredicate::Prefix("system/".into()),
            ),
            Capability::for_family(
                effect_ct::STORE_RESOLVE,
                ResourcePredicate::Prefix("system/".into()),
            ),
        ])
    }

    #[tokio::test]
    async fn a_published_compiler_pointer_resolves_to_a_runnable_artifact() {
        let Some(component) = reducer_component_bytes() else {
            eprintln!(
                "SKIP a_published_compiler_pointer_resolves_to_a_runnable_artifact: \
                 CDZ_LIVE_REDUCER_COMPONENT unset — set it to a real wasm reducer component (the nix build \
                 produces one) to exercise the publish→resolve→load→run arc."
            );
            return;
        };

        // The blob store holding compiled artifacts. `put` is content-addressed → the hash we publish is the
        // hash the resolve hands back, and blob-get at it returns these exact bytes.
        let mut blobs = MemBlobStore::new();
        // Content hash computed once + supplied to put (put no longer computes/returns it).
        let artifact_hash = Hash::of(&component);
        blobs
            .put(artifact_hash, bytes::Bytes::from(component.clone()))
            .await
            .expect("put the compiled wasm component into the blob store");

        // PUBLISH + RESOLVE (one session, two phases) over its own per-session NameStore; a system/-prefix
        // grant authorizes.
        let mut session = HostedSession::genesis(
            Hash::of(b"compiler-publisher-v1"),
            Box::new(PublishThenResolve { artifact_hash }),
            Box::new(publisher_authz()),
            CompositeExecutor::new(),
        )
        .with_name_store(NameStore::new());

        session.deliver(publish_go(), None).await.unwrap();
        assert_eq!(session.open_effects(), 0, "both store effects settled");

        let resolved_hex = session
            .session()
            .kv()
            .get(b"resolved")
            .expect("the agent recorded the resolved hash");
        assert_eq!(
            resolved_hex,
            artifact_hash.to_hex().as_bytes(),
            "COMPILER_LATEST resolved to the published artifact's hash"
        );

        // CONSUME: blob-get the bytes at the resolved hash + prove they're RUNNABLE — load as an
        // AsyncComponentReducer (loading validates the component + binds fold.apply), then RUN one fold turn.
        let fetched = blobs
            .get(&artifact_hash)
            .await
            .expect("blob-get succeeds")
            .expect("the resolved hash is present in the blob store");
        assert_eq!(
            fetched, component,
            "blob-get returned the exact published bytes"
        );

        let resolved_reducer = AsyncComponentReducer::from_component_bytes(&fetched).expect(
            "the resolved artifact loads as a runnable reducer component (fold.apply bound)",
        );
        let mut running = HostedSession::genesis(
            artifact_hash,
            Box::new(resolved_reducer),
            Box::new(Authorizer::new(vec![])),
            CompositeExecutor::new(),
        );
        // Bounded (FOLD_RUN_TIMEOUT): a runaway live reducer surfaces as a timeout, not a hung suite.
        tokio::time::timeout(FOLD_RUN_TIMEOUT, running.deliver(publish_go(), None))
            .await
            .expect("the resolved artifact's fold turn completes within FOLD_RUN_TIMEOUT (not a runaway loop)")
            .expect("the resolved artifact runs one fold turn to quiescence (no panic, §17)");
        assert_eq!(
            running.open_effects(),
            0,
            "the resolved artifact's turn settled (any emitted effects resolved/denied — it ran)"
        );
    }

    /// PUBLISHER: on inbound, `store/set`s the pointer → the artifact hash it was built with. One hop.
    struct Publisher {
        artifact_hash: Hash,
    }
    #[async_trait::async_trait(?Send)]
    impl Reducer for Publisher {
        async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
            if matches!(event.body, EventBody::Inbound { .. }) {
                let payload = encode_name_set(PUBLISH_POINTER, &self.artifact_hash);
                FoldOutput::with(vec![EffectRequest::new_with_family(
                    effect_ct::STORE_SET,
                    PUBLISH_POINTER,
                    Some(Payload::Inline(payload.into())),
                    Timeliness::Interactive,
                )])
            } else {
                FoldOutput::none()
            }
        }
    }

    /// CONSUMER: on inbound, `store/resolve`s the pointer; on the result records the resolved hash's hex in KV.
    struct Consumer;
    #[async_trait::async_trait(?Send)]
    impl Reducer for Consumer {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    FoldOutput::with(vec![EffectRequest::new_with_family(
                        effect_ct::STORE_RESOLVE,
                        PUBLISH_POINTER,
                        None,
                        Timeliness::Interactive,
                    )])
                }
                EventBody::EffectResult {
                    result: EffectOutcome::Ok(Some(Payload::Inline(bytes))),
                    ..
                } => {
                    if let Ok((_n, h)) = decode_name_set(bytes) {
                        kv.put(b"resolved".to_vec(), h.to_hex().into_bytes());
                    }
                    FoldOutput::none()
                }
                _ => FoldOutput::none(),
            }
        }
    }

    fn set_system() -> Authorizer {
        Authorizer::new(vec![]).with_family_grants(vec![Capability::for_family(
            effect_ct::STORE_SET,
            ResourcePredicate::Prefix("system/".into()),
        )])
    }
    fn resolve_system() -> Authorizer {
        Authorizer::new(vec![]).with_family_grants(vec![Capability::for_family(
            effect_ct::STORE_RESOLVE,
            ResourcePredicate::Prefix("system/".into()),
        )])
    }

    #[tokio::test]
    async fn a_consumer_agent_resolves_and_runs_what_a_separate_publisher_agent_published() {
        let Some(component) = reducer_component_bytes() else {
            eprintln!(
                "SKIP a_consumer_agent_resolves_and_runs_what_a_separate_publisher_agent_published: \
                 CDZ_LIVE_REDUCER_COMPONENT unset — set it to a real wasm reducer component to exercise the \
                 true 2-agent publish→consume loop."
            );
            return;
        };

        // The shared artifact store (host-owned). `put` is content-addressed → the hash the publisher writes
        // is the hash the consumer resolves, and blob-get at it returns these exact bytes.
        let mut blobs = MemBlobStore::new();
        let artifact_hash = Hash::of(&component);
        blobs
            .put(artifact_hash, bytes::Bytes::from(component.clone()))
            .await
            .expect("put the wasm artifact");

        // (1) PUBLISHER: session A sets COMPILER_LATEST → artifact_hash into its OWN per-session store.
        let mut publisher = HostedSession::genesis(
            Hash::of(b"publisher-agent-v1"),
            Box::new(Publisher { artifact_hash }),
            Box::new(set_system()),
            CompositeExecutor::new(),
        )
        .with_name_store(NameStore::new());
        publisher.deliver(publish_go(), None).await.unwrap();
        assert_eq!(publisher.open_effects(), 0, "the publish set settled");

        // (2)+(3) HOST BRIDGE: read A's store back out, export its set-event stream, replay it into a fresh
        // store for the consumer — the explicit host-owned sharing policy (no shared handle; kernel stays
        // share-free).
        let published = publisher
            .session()
            .name_store()
            .expect("the publisher has a name store attached")
            .to_set_entries();
        assert!(
            published
                .iter()
                .any(|(n, h)| n == PUBLISH_POINTER && *h == artifact_hash),
            "the publisher's store carries COMPILER_LATEST → artifact_hash"
        );
        let consumer_store =
            NameStore::replay_set_entries(published.iter().map(|(n, h)| (n.as_str(), *h)))
                .expect("replay the published set-stream into the consumer's store");

        // (4) CONSUMER: session B (a DIFFERENT agent, resolve-only grant) resolves the pointer A published.
        let mut consumer = HostedSession::genesis(
            Hash::of(b"consumer-agent-v1"),
            Box::new(Consumer),
            Box::new(resolve_system()),
            CompositeExecutor::new(),
        )
        .with_name_store(consumer_store);
        consumer.deliver(publish_go(), None).await.unwrap();
        assert_eq!(consumer.open_effects(), 0, "the resolve settled");

        let resolved_hex = consumer
            .session()
            .kv()
            .get(b"resolved")
            .expect("the consumer recorded the resolved hash");
        assert_eq!(
            resolved_hex,
            artifact_hash.to_hex().as_bytes(),
            "the consumer resolved COMPILER_LATEST to the exact hash the publisher set (cross-agent)"
        );

        // ...and the resolved artifact RUNS: blob-get the bytes at that hash, load as a reducer, fold a turn.
        let fetched = blobs
            .get(&artifact_hash)
            .await
            .expect("blob-get succeeds")
            .expect("the resolved hash is present in the shared blob store");
        let resolved_reducer = AsyncComponentReducer::from_component_bytes(&fetched)
            .expect("the artifact the consumer resolved loads as a runnable reducer");
        let mut running = HostedSession::genesis(
            artifact_hash,
            Box::new(resolved_reducer),
            Box::new(Authorizer::new(vec![])),
            CompositeExecutor::new(),
        );
        tokio::time::timeout(FOLD_RUN_TIMEOUT, running.deliver(publish_go(), None))
            .await
            .expect("the resolved artifact's fold turn completes within FOLD_RUN_TIMEOUT (not a runaway)")
            .expect("the artifact the consumer resolved runs one fold turn to quiescence (no panic, §17)");
        assert_eq!(
            running.open_effects(),
            0,
            "the resolved artifact's turn settled — publisher published, a separate consumer resolved + RAN it"
        );
    }
}
