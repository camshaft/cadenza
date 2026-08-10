//! Effects and capabilities — what a reducer can *ask the world to do*, and what it's *allowed* to.
//!
//! A reducer never touches the world directly (§3, §5): it emits **effect requests** and the kernel
//! executes them, folding results back as events. Two review findings shape these types and MUST NOT
//! be softened:
//!
//! - **SEC-F1 (resource-scoped capabilities):** the effect *kind* alone (`Http.get`) is NOT a
//!   sufficient permission — the security-relevant distinction lives in the *target argument*
//!   (`http://169.254.169.254/…` IMDS vs. an allowed host). So a `Capability` is `(kind, predicate)`
//!   and authorization checks the predicate against the *resolved* target of each request.
//! - **S4 (effect-id-keyed continuations):** every dispatched effect has a kernel-assigned `EffectId`;
//!   the reducer resumes when the *result event* carrying that id arrives. Correlation is by id, so
//!   concurrent / out-of-order results are unambiguous.

use crate::event::ContentType;
use crate::hash::Hash;

/// A kernel-assigned identifier for a single dispatched effect, unique within a session. The reducer
/// stores its continuation keyed by this (§16c-S4) and the result/timeout event carries it back.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct EffectId(pub u64);

/// The kind of effect — the coarse verb. Target/args live in [`EffectRequest`]. This is deliberately a
/// small, explicit enum for v0 (design §15b: a handful of local effects); it grows as executors land.
/// `Hash` so it can key a by-kind executor router ([`crate::executor::CompositeExecutor`]).
///
/// Migration note (extensible content-typed effects, operator seq-39): each kind has a canonical
/// lowercase FAMILY string ([`EffectKind::family`] / [`effect_ct`]) — the same string the codec
/// (event_ast) already writes and the Cedar authorizer's action-name uses. That family is the seam the
/// effect model migrates onto: routing/authz key on the family string (so a NEW effect type is added
/// without growing this enum + a kernel edit), and this enum stays the canonical family source until the
/// migration replaces it with a raw content-type tag.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum EffectKind {
    /// Run a shell command. Target = the program + args (see `EffectRequest::target`).
    Shell,
    /// An HTTP request. Target = the URL (host is what the capability predicate gates — SEC-F1).
    Http,
    /// Invoke a model. Target = the model id.
    Model,
    /// Read the wall clock. Result is a recorded `time-result` (§9c) — the reducer never reads the
    /// clock directly.
    Now,
    /// Arm a timer. The kernel injects a `timer-fired` event at the (absolute) deadline (§9c/§16c-S5).
    Timer,
    /// Send a signal to a peer session's inbox (§5). Target = the session id.
    Emit,
}

/// The canonical lowercase FAMILY strings for the well-known effect kinds — the extensible-effects vocab
/// (operator seq-39). These are the SINGLE source of truth for the family names the codec writes
/// (event_ast `kind_atom`/`read_kind`), the executor router keys on, and the Cedar authorizer maps to a
/// policy ACTION — so they can't drift on a typo across those sites. A new effect type is a NEW family
/// string here (routed to a handler by string), never a new [`EffectKind`] variant + a kernel recompile.
pub mod effect_ct {
    pub const SHELL: &str = "shell";
    pub const HTTP: &str = "http";
    pub const MODEL: &str = "model";
    pub const NOW: &str = "now";
    pub const TIMER: &str = "timer";
    pub const EMIT: &str = "emit";

    /// The `control/*` namespace PREFIX (control-plane / register-by-string design). A family whose string
    /// starts with this is a CONTROL family: authz-EXEMPT and NEVER routed to an executor — the kernel/host
    /// answers it in-process (asking "what may I do" is not itself a world-action, and gating it would be
    /// circular). The well-known EFFECT families above stay BARE (no `effect/` prefix) because they are a
    /// DURABLE WIRE VALUE (the codec writes `EffectKind::family()` into the log + the Cedar action-map), so
    /// renaming them would break log compatibility. So the partition is asymmetric: control families carry
    /// this prefix (they are all new — no wire history), effect families are bare. See [`is_control_family`].
    pub const CONTROL_PREFIX: &str = "control/";

    /// The well-known `control/capabilities` family — the host-capability-discovery query (a `ControlKernel`
    /// disposition: the kernel answers it inline via `project_manifest`). The first control family.
    pub const CAPABILITIES: &str = "control/capabilities";

    /// The well-known `control/summary` family — the fork-for-query summarize control effect (a
    /// `ControlHostSurfaced` disposition: returned to the driver, captured by the query fork's watch).
    pub const SUMMARY: &str = "control/summary";

    /// The well-known `control/signature` family — the component SIGNATURE-QUERY (composable-component-calls
    /// part-1, greenlit 2026-08-07): a reducer asks "given this component (by hash/name), what are its
    /// exported funcs + their param/result types?" so a Cadenza orchestration program can DISCOVER a target's
    /// callable surface before invoking it. CONTROL-PLANE (authz-EXEMPT, like `control/capabilities`) —
    /// introspection is not a world-action, so it isn't gated (only the actual INVOKE keeps per-call authz).
    /// A `ControlHostSurfaced` disposition like `control/summary`: the HOST answers it (reflecting the target's
    /// `component_type().exports()` — wasmtime, host-side) + folds back a canonical [`component-signature`
    /// descriptor](crate::event_ast::encode_component_signature) as the result; the kernel only carries the
    /// family vocab + owns the descriptor CODEC (the reducer↔host wire shape). The target component (hash/name)
    /// rides the effect target. V0 carries each param/result TYPE as OPAQUE wit-bytes (a reducer discovers
    /// export names + arities + type-bytes = enough to route/dispatch); a follow-up increment lowers the
    /// wit-types to a Cadenza-decodable Ast (v-metaprogramming owns that type mapping).
    pub const SIGNATURE: &str = "control/signature";

    /// The `store/*` namespace PREFIX — the §4c global-store WRITE layer (mutable name→hash pointers). A
    /// family whose string starts with this is a STORE effect: unlike `control/*` (authz-EXEMPT), a store
    /// effect is AUTHZ-GATED — a `store/set` is the anti-hijack surface (repointing `system/compiler/latest`
    /// at an evil hash), so it goes through the authorizer keyed on the NAME's prefix authority (a Cedar
    /// prefix-grant / [`crate::name_store::NameStore::authority_prefix_of`], §4c point 2). These are all NEW
    /// families (no wire history), so they carry the `store/` prefix — the same asymmetry as `control/`.
    pub const STORE_PREFIX: &str = "store/";

    /// `store/set` — append `set(name, hash)` to a mutable name's value-over-time log (§4c point 1). The
    /// effect target is the mutable NAME; the hash rides the payload ([`crate::event_ast::encode_name_set`]).
    /// AUTHZ-GATED by the name's prefix authority (only a `system/` grant may `store/set` a `system/…` name).
    pub const STORE_SET: &str = "store/set";

    /// `store/resolve` — read a mutable name's CURRENT hash = its latest `set` (§4c point 3: resolving
    /// FREEZES the resolved hash into the resolver's log, so a later hijacking set can't retroactively
    /// change it). A read; a broad store grant admits it (the write-authority gate is on `store/set`).
    pub const STORE_RESOLVE: &str = "store/resolve";

    /// `store/add` — join a member to a GROUP name's OR-set (§4c session-directory I3). The effect target is
    /// the mutable GROUP NAME; the member hash + its unique add-tag `(origin, seq)` ride the payload (a
    /// `member-op` blob, [`crate::event_ast::encode_member_op`]). AUTHZ-GATED on the name's prefix authority,
    /// exactly like `store/set` (the write-authority gate is on the GROUP name). A group name is a pointer
    /// XOR a group — [`crate::name_store::NameStore::add_op`] refuses a name already used as a single-value
    /// pointer with [`NameStoreError::NameModeMismatch`](crate::name_store::NameStoreError::NameModeMismatch).
    pub const STORE_ADD: &str = "store/add";

    /// `store/remove` — retract a member from a GROUP name's OR-set (§4c session-directory I3). Observed-remove
    /// (add-wins): the payload's tag names the SPECIFIC add being cleared; a concurrent re-add with a FRESH
    /// tag survives. Same target/payload/authz shape as [`STORE_ADD`] (the op's `add` flag is `false`).
    pub const STORE_REMOVE: &str = "store/remove";

    /// `store/resolve-all` — read a GROUP name's CURRENT membership: fold its OR-set log add-wins into a
    /// deterministic ascending-hash member set (§4c D1, [`crate::name_store::NameStore::resolve_all`]). A pure
    /// READ (no payload) — the group analogue of [`STORE_RESOLVE`]; a broad store grant admits it. The frozen
    /// member set is what a multicast fan-out (§8) iterates, so its order is byte-stable.
    pub const STORE_RESOLVE_ALL: &str = "store/resolve-all";

    /// Is `family` a GROUP OR-set store verb (`store/add` / `store/remove` / `store/resolve-all`, §4c
    /// session-directory I3) — as opposed to the single-value pointer verbs (`store/set` / `store/resolve`)?
    /// Both partitions share the [`STORE_PREFIX`] (both are authz-gated on the name), but they carry a
    /// DIFFERENT payload shape (a `member-op` blob vs a `name-set` blob) and dispatch to a different store
    /// method ([`crate::name_store::NameStore::apply_group_effect`] vs `apply_effect`), so the kernel's
    /// store arm routes on this sub-partition. `false` for a non-store family (check [`is_store_family`] first).
    pub fn is_group_store_family(family: &str) -> bool {
        matches!(family, STORE_ADD | STORE_REMOVE | STORE_RESOLVE_ALL)
    }

    /// Is `family` in the `store/*` namespace (the §4c global-store write layer, authz-gated on the name's
    /// prefix authority)? The one-source prefix test the drive loop applies alongside [`is_control_family`]
    /// to route `store/*` to the attached name-store (via `Session::apply_store_effect`) rather than a
    /// generic executor — after the SEC-F1 authorize gate, since store writes ARE authz-gated (§4c slice 3b).
    pub fn is_store_family(family: &str) -> bool {
        family.starts_with(STORE_PREFIX)
    }

    /// The `lifecycle/*` namespace PREFIX — the §lifecycle session-control partition (spawn/suspend/resume/
    /// terminate). UNLIKE `store/*` (kernel-applied to the name-store) and `control/*` (authz-exempt,
    /// kernel/host-answered), a `lifecycle/*` effect is AUTHZ-GATED and routes through the NORMAL
    /// authorize→executor path: the HOST registers an executor that `handles_family` (the `Executor` trait
    /// method) the lifecycle names + defers the session-registry mutation to the loop (an executor can't
    /// hold `&mut AgentHost` while the driven session borrows the registry — the on-loop-no-deadlock
    /// design). So the kernel needs NO special drive-loop arm for it — only these family-string consts (one
    /// source of truth across a reducer, the host executor, and the manifest). Authority is enforced via the
    /// `FamilyGrant` seam (`Capability::for_family` + `crate::authz::Authorizer::with_family_grants`): a grant
    /// keyed on a `lifecycle/*` family string + a `ResourcePredicate` over the target (the target SessionId).
    /// This does NOT yet restrict a controller to its transitive `Spawned`-descendants — a supervision-tree
    /// (descendant-only) restriction needs a host/Cedar tree-walk over the spawn edges (host-registry state
    /// the kernel's static predicate can't compute), so lifecycle authority is currently target-predicate
    /// scoped, not tree-scoped. Register-by-string (`Emit` placeholder kind).
    pub const LIFECYCLE_PREFIX: &str = "lifecycle/";

    /// `lifecycle/spawn` — spawn a durable CHILD session (target = the child's reducer hash; the effect
    /// result is the child's SessionId = its genesis hash). The host executor instantiates the child +
    /// registers it + records the parent→child [`Spawned`](crate::event::EventBody::Spawned) edge via
    /// [`Session::record_spawn`](crate::kernel::Session::record_spawn).
    pub const LIFECYCLE_SPAWN: &str = "lifecycle/spawn";

    /// `lifecycle/suspend` — stop scheduling a target session's ticks (target = SessionId). Durable log
    /// untouched (suspend is a host-scheduler state, not a kernel event); queued inbound is held.
    pub const LIFECYCLE_SUSPEND: &str = "lifecycle/suspend";

    /// `lifecycle/resume` — re-enable a suspended session (target = SessionId); replays any held inbound.
    pub const LIFECYCLE_RESUME: &str = "lifecycle/resume";

    /// `lifecycle/terminate` — terminate a target session (target = SessionId). The host executor drives
    /// [`Session::terminate`](crate::kernel::Session::terminate) (the durable [`Terminated`](crate::event::EventBody::Terminated)
    /// marker + fold-refusal), removes it from the registry, and bounces in-flight Emits to it as a
    /// permanent Failure-to-sender.
    pub const LIFECYCLE_TERMINATE: &str = "lifecycle/terminate";

    /// Is `family` in the `lifecycle/*` namespace (the §lifecycle session-control partition)? A prefix
    /// test (like [`is_store_family`]). Note this partition is executor-routed (not kernel-handled), so
    /// the drive loop needs no special arm keyed on this — it's here for the manifest + discoverability +
    /// so a future kernel-side use has one source of truth for the partition boundary.
    pub fn is_lifecycle_family(family: &str) -> bool {
        family.starts_with(LIFECYCLE_PREFIX)
    }

    /// The `fs/*` namespace PREFIX — the §GAP-3 filesystem partition (a first-class file effect so an agent
    /// edits code SAFELY + gate-ably, rather than shelling out to `sed`). Like `lifecycle/*` it is AUTHZ-GATED
    /// and EXECUTOR-ROUTED through the normal authorize→executor path: the HOST registers a thin `FsExecutor`
    /// that `handles_family` the fs names (read/write/glob = the irreducible syscalls), so the kernel needs no
    /// drive-loop arm, only these family consts (one source of truth across a reducer, the host executor, and
    /// the manifest). Authority is a `FamilyGrant` (`Capability::for_family`) whose `ResourcePredicate` gates
    /// the effect TARGET = the resolved PATH: path-scoping is an evolvable Cedar policy
    /// (`permit(action=="fs/write") when resource.target like "implementation/**"`), NOT a host allow-list —
    /// the minimize-host-logic standing order, same lesson as the shell allow-list. Register-by-string
    /// (`Emit` placeholder kind). NO path allow-list in the kernel OR host; Cedar owns the path policy.
    pub const FS_PREFIX: &str = "fs/";

    /// `fs/read` — read a file's bytes (target = the path; result = the file contents as an inline payload).
    pub const FS_READ: &str = "fs/read";

    /// `fs/write` — create-or-overwrite a file (target = the path; payload = the bytes to write). The
    /// agent-edit loop is `fs/read` → modify in the reducer → `fs/write`; a dedicated `fs/edit` may come later.
    pub const FS_WRITE: &str = "fs/write";

    /// `fs/glob` — list paths matching a glob / under a directory (target = the glob or dir; result = the
    /// matching paths). Lets an agent discover files to read/edit within its Cedar-granted path scope.
    pub const FS_GLOB: &str = "fs/glob";

    /// Is `family` in the `fs/*` namespace (the §GAP-3 filesystem partition, authz-gated on the resolved
    /// PATH target)? A prefix test (like [`is_lifecycle_family`]/[`is_store_family`]). Executor-routed, so
    /// the drive loop needs no special arm — here for the manifest + discoverability + one source of truth.
    pub fn is_fs_family(family: &str) -> bool {
        family.starts_with(FS_PREFIX)
    }

    /// The `metric/*` namespace PREFIX — the reducer METRICS-PUBLISH partition (operator Q3): a reducer emits
    /// a metric that the HOST forwards to its existing metric backends (statsd/otlp/prometheus). Like
    /// `fs/*`/`lifecycle/*` it is AUTHZ-GATED + EXECUTOR-ROUTED (register-by-string, `Emit` placeholder kind,
    /// no kernel drive-loop arm): the host registers a `MetricExecutor` (mirroring `EmitExecutor`) that
    /// `handles_family` the metric names, records into the shared metrics Registry, and applies its OWN
    /// cardinality bound + guest-string safety (host concern, not kernel). The kernel only carries the metric
    /// payload bytes (the `metric-publish` codec) + the family vocab. Metric SEMANTICS (what to measure, when)
    /// live in the reducer — the minimize-kernel/host-logic standing order: policy on the log, not baked in.
    pub const METRIC_PREFIX: &str = "metric/";

    /// `metric/publish` — publish one metric sample (target = the metric name; payload = the
    /// `metric-publish` blob via [`crate::event_ast::encode_metric_publish`]: name + kind + value + labels).
    /// The host `MetricExecutor` records it into the metrics Registry; a broad `metric/*` grant admits it.
    pub const METRIC_PUBLISH: &str = "metric/publish";

    /// Is `family` in the `metric/*` namespace (the reducer metrics-publish partition)? A prefix test (like
    /// [`is_fs_family`]). Executor-routed, so the drive loop needs no special arm — here for the manifest +
    /// discoverability + one source of truth for the partition boundary.
    pub fn is_metric_family(family: &str) -> bool {
        family.starts_with(METRIC_PREFIX)
    }

    /// The `blob/*` namespace PREFIX — the reducer CONTENT-ADDRESSED STORE partition (cadenza-docs I3, but a
    /// GENERIC dep for any content reducer, not doc-specific). A reducer PUTs bytes into the CAS and gets
    /// back their content [`crate::hash::Hash`] (the address), or GETs bytes back by hash. The `blob.rs`
    /// [`crate::blob::BlobStore`] is the storage PRIMITIVE (`put`/`get`); this family is the reducer-facing
    /// EFFECT that invokes it — the missing piece a reducer needs, since a reducer emits effects, it can't
    /// call `BlobStore::put` directly. Like `fs/*`/`metric/*` it is AUTHZ-GATED + EXECUTOR-ROUTED
    /// (register-by-string, `Emit` placeholder kind, no kernel drive-loop arm): the HOST registers a
    /// blob executor over its `BlobStore` backend, Cedar-gated. Content-addressed → integrity is free (the
    /// key IS the hash); immutable by construction (same bytes → same key, a put is idempotent). The canonical
    /// doc-publish path is `blob/put` (doc-AST bytes → Hash) + `store/set doc/<pkg>` (register the hash by
    /// name) — NOT `fs/write` (which rejects a blob-ref payload; a different, path-addressed store).
    pub const BLOB_PREFIX: &str = "blob/";

    /// `blob/put` — store bytes in the content-addressed store, returning their content [`crate::hash::Hash`]
    /// (the address) as the effect result. Payload = the bytes to store; the result-hash is what the reducer
    /// registers (e.g. `store/set doc/<pkg>`) or embeds. Idempotent (content-addressed: the same bytes always
    /// map to the same key). The HOST blob executor performs the `BlobStore::put`.
    pub const BLOB_PUT: &str = "blob/put";

    /// `blob/get` — fetch bytes from the content-addressed store by hash (target = the hash; result = the
    /// bytes, or an absent-blob outcome). A self-verifying backend re-hashes the returned bytes to the
    /// requested key (content-addressing makes tamper-detection free). The HOST blob executor performs the
    /// `BlobStore::get`.
    pub const BLOB_GET: &str = "blob/get";

    /// Is `family` in the `blob/*` namespace (the reducer content-addressed-store partition)? A prefix test
    /// (like [`is_fs_family`]/[`is_metric_family`]). Executor-routed, so the drive loop needs no special arm —
    /// here for the manifest + discoverability + one source of truth for the partition boundary.
    pub fn is_blob_family(family: &str) -> bool {
        family.starts_with(BLOB_PREFIX)
    }

    /// The `ws/*` namespace PREFIX — THE OUTPOST O1: a gateway host where peer agents connect over a
    /// websocket + a GUEST reducer routes/federates them. The namespace spans BOTH directions. (1) The
    /// OUTBOUND EFFECT [`WS_SEND`] — a reducer sends a frame to a peer — authz-gated + executor-routed
    /// (register-by-string, `Emit` placeholder kind, no drive-loop arm), like `fs/*`/`blob/*`. (2) INBOUND
    /// EVENTS the host emits onto the log so the reducer folds connection lifecycle + traffic into its
    /// federation state: a ws data frame arrives as a plain `Inbound` event, and — per operator directive on
    /// #2804 — the transport also emits [`WS_CONNECT`]/[`WS_DISCONNECT`] `Inbound` events when a peer
    /// connection is ESTABLISHED / CLOSED, so the lifecycle is DURABLE (auditable — a retrospective agent
    /// sees when peers joined/left) and the reducer POLICY can react (learn a peer exists, address `ws/send`
    /// to it, prune on disconnect). The reducer opens NO connection (the PEER opens it; the host mints an
    /// opaque conn-id on accept) — so there is no `ws/open`; a reducer-initiated `ws/close` EFFECT is a later
    /// additive op the prefix reserves room for. Routing/federation/tool-visibility POLICY lives entirely in
    /// the guest reducer (minimize-host-logic: the host is plumbing).
    pub const WS_PREFIX: &str = "ws/";

    /// `ws/send` — (OUTBOUND EFFECT) write a frame to a connected peer. `target` = the opaque connection-id
    /// the host minted on accept and handed the reducer in the `Inbound` framing (the reducer ECHOES it back —
    /// the kernel never interprets it, exactly like a `shell` program-target or an `Emit` peer-id rides
    /// `req.target` as an opaque `Arc<str>`); `payload` = the outbound frame bytes. The HOST ws executor maps
    /// the conn-id → its live connection and writes the frame, folding an Ok(delivered)/Err(gone) outcome
    /// back. Per-send Cedar authz gates `resource.target = <conn-id>` (a policy can scope which peers/
    /// peer-classes a reducer may send to). NO new kernel type — conn-id is a string target, frame is bytes.
    pub const WS_SEND: &str = "ws/send";

    /// `ws/dial` — (OUTBOUND EFFECT, hub-federation F0-effect) a reducer's fold DECIDES to federate and dials
    /// a hub: `target` = the hub URL (opaque UTF-8, like a `shell` program-target or an `Emit` peer-id — the
    /// kernel never interprets it), authz-gated per-dial exactly like [`WS_SEND`] (`resource.target = <hub-url>`,
    /// the SSRF/egress guard — a session may dial only URLs its capability grants). Dispatched-WITH-result: the
    /// HOST `WsDialExecutor` spawns the dial (the landed `ws_dial::dial_hub` transport primitive) and folds the
    /// minted conn-id hex back as the effect result, so the reducer binds conn-id↔hub and can then address
    /// [`WS_SEND`] to it. A grantable outbound effect (in [`ALL`]), unlike the host-minted inbound
    /// [`WS_CONNECT`]/[`WS_DISCONNECT`] events. NO new kernel type — url is a string target, conn-id is bytes.
    pub const WS_DIAL: &str = "ws/dial";

    /// `ws/connect` — (INBOUND EVENT `content_type.family`, operator directive #2804) the host emits this
    /// `Inbound` when a peer websocket connection is ESTABLISHED; `payload` = the opaque conn-id bytes the
    /// host minted on accept (v0 — a peer-descriptor blob may ride later). The reducer folds it to learn the
    /// peer exists + may now address `ws/send` to that conn-id. Host-minted + reducer-matched (an inbound
    /// family, NOT a reducer-emitted effect — so it is NOT in [`ALL`], the grantable-effect set); named as a
    /// const for ONE source of truth (host + reducer share the exact string, safe-logging classified).
    pub const WS_CONNECT: &str = "ws/connect";

    /// `ws/disconnect` — (INBOUND EVENT `content_type.family`, operator directive #2804) the host emits this
    /// `Inbound` when a peer connection CLOSES; `payload` = the conn-id bytes (a close-reason may ride later).
    /// The reducer folds it to prune that peer from its federation state (a subsequent `ws/send` to it folds
    /// Err(gone)). Inbound family, host-minted + reducer-matched (NOT in [`ALL`]); const for one source of truth.
    pub const WS_DISCONNECT: &str = "ws/disconnect";

    /// Is `family` in the `ws/*` namespace (the reducer outbound-websocket partition, THE OUTPOST)? A prefix
    /// test (like [`is_fs_family`]/[`is_blob_family`]). Executor-routed, so the drive loop needs no special arm —
    /// here for the manifest + discoverability + one source of truth for the partition boundary.
    pub fn is_ws_family(family: &str) -> bool {
        family.starts_with(WS_PREFIX)
    }

    /// The `effect/` name-server registration PREFIX (userspace-effects I1) — NOT an effect family itself, but
    /// the GNS namespace where a userspace effect family is registered: `effect/<family>` is a pointer name
    /// whose value is the handler session's `SessionId` (genesis hash). A session Cedar-granted `store/set`
    /// over `effect/<family>` CLAIMS/repoints that family to itself (the anti-hijack write authority, see
    /// [`crate::name_store::NameAuthority::Effect`]); the delegating executor resolves it via
    /// [`crate::name_store::NameStore::resolve_effect_handler`]. This is the store-NAME prefix, distinct from
    /// the effect FAMILY string a reducer emits (the family `weather` registers AT `effect/weather`).
    pub const EFFECT_REGISTRY_PREFIX: &str = "effect/";

    /// `effect/reply` — a ROUTED OUTBOUND effect family a userspace-effect HANDLER emits to answer a request
    /// it was forwarded (userspace-effects I4). `target` = the opaque reply-token the delegating executor
    /// minted + put in the forwarded request's framing (bound to the original `(caller SessionId, EffectId)`);
    /// `payload` = the response bytes. The HOST `ReplyExecutor` validates+consumes the token and calls
    /// [`crate::kernel::Session::settle_effect_result`] on the ORIGINAL caller's open (Deferred) effect —
    /// closing the request→forward→reply→settle loop. Executor-routed + per-effect authz on the token (a
    /// handler may only reply to a token it holds); a grantable effect (in [`ALL`]) + safe-logging. NOTE this
    /// shares the `effect/` prefix with [`EFFECT_REGISTRY_PREFIX`] but is a DISTINCT thing: `effect/<family>`
    /// is a store-NAME (the registration pointer), `effect/reply` is an EFFECT FAMILY (a routed verb) — so it
    /// is explicitly EXCLUDED from [`is_registered_effect_family`] (it is a built-in, never a userspace family).
    pub const EFFECT_REPLY: &str = "effect/reply";

    /// Is `family` a candidate USERSPACE-EFFECT family — i.e. NOT one of the built-in well-known partitions
    /// (so it would route to a registered handler, if one is registered)? A SYNTACTIC check: an emitted
    /// effect whose family is not a kernel built-in (EffectKind / control/store/lifecycle/fs/metric/blob/ws
    /// / the `effect/reply` routed family) is a candidate for `effect/<family>` handler resolution. Whether a
    /// handler is ACTUALLY registered is the runtime lookup
    /// [`crate::name_store::NameStore::resolve_effect_handler`] (mechanism = "resolves in the registry"); this
    /// predicate is the partition boundary the drive loop / delegating executor uses to decide "try the
    /// userspace-effect path" vs "a known built-in family". Fail-safe: a family that IS a built-in returns
    /// false (built-ins are never shadowed by a userspace handler).
    pub fn is_registered_effect_family(family: &str) -> bool {
        // A built-in well-known family is NOT a userspace effect (built-ins win — no shadowing). `effect/reply`
        // shares the `effect/` prefix but is a built-in ROUTED family (the handler's reply verb), NOT a
        // userspace registration target — exclude it explicitly so it never mis-routes to handler resolution.
        let is_builtin = super::EffectKind::from_family(family).is_some()
            || is_control_family(family)
            || is_store_family(family)
            || is_lifecycle_family(family)
            || is_fs_family(family)
            || is_metric_family(family)
            || is_blob_family(family)
            || is_ws_family(family)
            || family == EFFECT_REPLY;
        !is_builtin && !family.is_empty()
    }

    /// Is `family` in the `control/*` namespace (authz-exempt, host/kernel-answered, never executor-routed)?
    /// The partition test the drive loop applies BEFORE authorize/route: `true` → control path, `false` →
    /// the effect path (authorize → executor). A simple prefix check on [`CONTROL_PREFIX`] — one source of
    /// truth for the split, so the drive loop and the registry can't disagree on what "control" means.
    pub fn is_control_family(family: &str) -> bool {
        family.starts_with(CONTROL_PREFIX)
    }

    /// Is `family` EXEMPT from the SEC-F1 capability authz gate? A control family (host-answered, never
    /// executor-routed) OR [`EFFECT_REPLY`]. This is the authz-gate test ONLY — it does NOT change routing:
    /// a control family still takes the control path, and `effect/reply` is still EXECUTOR-routed to the host
    /// `ReplyExecutor`; exemption only means the drive loop skips the `authorize()` call for it.
    ///
    /// Why `effect/reply` is exempt (userspace-effects D2, and the [`EFFECT_REPLY`] doc): its `target` is an
    /// opaque 32-byte reply-TOKEN (not UTF-8), so a `FamilyGrant`/`Capability` predicate — which matches on
    /// `req.target_str()` (UTF-8) — CANNOT admit it (`target_str` Errs → `permits` is false → the reply is
    /// wrongly `AuthzDenied` and the caller never resumes). And capability-gating would be REDUNDANT anyway:
    /// the host `ReplyExecutor` cryptographically validates+consumes the unforgeable one-shot token (a handler
    /// may only reply to a token it holds; forged/stale/double-settle are refused), which is STRICTLY STRONGER
    /// than a capability grant on the opaque target. Exemption is NOT a security hole: the reply is still
    /// executor-routed, so with no `ReplyExecutor` it is an unhandled-effect error, never an unchecked action —
    /// the token gate lives in the executor, independent of this authz gate. If policy on `effect/reply` is
    /// ever wanted (I5 Cedar), it keys on the caller/handler identity, NOT the opaque token-target.
    pub fn is_authz_exempt(family: &str) -> bool {
        is_control_family(family) || family == EFFECT_REPLY
    }

    /// Is `family` a FOLD-BACK control family — a `control/*` whose answer must RESUME the emitting
    /// reducer's continuation, so the drive loop gives it a `Dispatched` frame (entering `open`, keyed by
    /// `EffectId` + carrying the token) and the host later settles it via
    /// [`crate::kernel::Session::settle_effect_result`]? Today just `control/signature`: the whole point of
    /// signature-query is the emitting reducer receives the descriptor to then route a call, so the reflected
    /// bytes fold BACK to the live session as an `EffectResult` (same shape as any routed effect). This is
    /// distinct from the other two control dispositions: `control/capabilities` is kernel-answered INLINE (it
    /// also dispatches, but the kernel produces the answer itself, no host round-trip); `control/summary` is
    /// FIRE-AND-FORGET fork-scrape (surfaced to the driver with NO `Dispatched` frame — the answer never
    /// returns to the live session, so it must NOT enter `open` or it would hang as a never-settled effect).
    /// Keying the selective dispatch on THIS predicate is what keeps summary's fork-scrape untouched.
    pub fn is_fold_back_control(family: &str) -> bool {
        family == SIGNATURE
    }

    /// If `family` is a WELL-KNOWN control-plane family, return its `&'static str` const (so a caller can
    /// hold it as a zero-alloc `Cow::Borrowed` instead of owning the string). `None` for an unknown/
    /// ad-hoc control family. The control-plane analogue of [`super::EffectKind::from_family`] — used by
    /// [`super::EffectRequest::new_with_family`] to keep the #1563/#1722 zero-alloc invariant for
    /// `control/capabilities` and `control/summary`, which have no `EffectKind`.
    pub fn wellknown_control(family: &str) -> Option<&'static str> {
        match family {
            CAPABILITIES => Some(CAPABILITIES),
            SUMMARY => Some(SUMMARY),
            SIGNATURE => Some(SIGNATURE),
            _ => None,
        }
    }

    /// The canonical, finite set of well-known effect families — the SAME set routing/authz/codec key on.
    /// Iterating it is what makes capability-manifest projection complete BY CONSTRUCTION (probe each known
    /// family; there is nothing to miss — see [`super::project_manifest`]). Keep in sync with the consts
    /// above (they're the single source; this just lists them for enumeration).
    pub const ALL: &[&str] = &[
        SHELL,
        HTTP,
        MODEL,
        NOW,
        TIMER,
        EMIT,
        LIFECYCLE_SPAWN,
        LIFECYCLE_SUSPEND,
        LIFECYCLE_RESUME,
        LIFECYCLE_TERMINATE,
        FS_READ,
        FS_WRITE,
        FS_GLOB,
        METRIC_PUBLISH,
        BLOB_PUT,
        BLOB_GET,
        WS_SEND,
        WS_DIAL,
        EFFECT_REPLY,
    ];

    /// If `family` is a WELL-KNOWN family whose string is a FIXED, kernel-defined `&'static` — one of the
    /// built-in effect verbs ([`ALL`], via [`super::EffectKind::from_family`]) or the exact control/store/fs/
    /// metric families (`control/capabilities`, `control/summary`, `store/set`, `store/resolve`, `store/add`,
    /// `store/remove`, `store/resolve-all`, `fs/read`, `fs/write`, `fs/glob`, `metric/publish`, `blob/put`,
    /// `blob/get`, `ws/send`, `ws/connect`, `ws/disconnect`, `effect/reply`) — return that canonical
    /// `&'static str`. `None` for an EXTENSION family (register-by-string). (`ws/connect`/`ws/disconnect` are
    /// inbound EVENT families, not effects, so they're here for safe-logging but NOT in [`ALL`].)
    ///
    /// The distinction matters for LOGGING (github-liaison #2180 residual): `ContentType.family` is a
    /// `Cow<'static, str>` and for an extension family it carries the CALLER's `Cow::Owned` verbatim — i.e.
    /// guest-controlled bytes that would leak off-box via the tracing subscriber (same class as the effect
    /// `target`, which #2180 already redacts). Only an EXACT well-known string is safe to emit; a PREFIX
    /// match (`is_control_family` / `is_store_family`) is NOT sufficient here — a guest can emit
    /// `store/<secret>` or `control/<secret>` that passes the prefix check while carrying guest bytes. So
    /// this matches the fixed strings exactly; the logger emits `Some(name)` verbatim and redacts a `None`
    /// family to its length.
    pub fn wellknown_static_str(family: &str) -> Option<&'static str> {
        if let Some(kind) = super::EffectKind::from_family(family) {
            return Some(kind.family());
        }
        match family {
            CAPABILITIES => Some(CAPABILITIES),
            SUMMARY => Some(SUMMARY),
            SIGNATURE => Some(SIGNATURE),
            STORE_SET => Some(STORE_SET),
            STORE_RESOLVE => Some(STORE_RESOLVE),
            STORE_ADD => Some(STORE_ADD),
            STORE_REMOVE => Some(STORE_REMOVE),
            STORE_RESOLVE_ALL => Some(STORE_RESOLVE_ALL),
            LIFECYCLE_SPAWN => Some(LIFECYCLE_SPAWN),
            LIFECYCLE_SUSPEND => Some(LIFECYCLE_SUSPEND),
            LIFECYCLE_RESUME => Some(LIFECYCLE_RESUME),
            LIFECYCLE_TERMINATE => Some(LIFECYCLE_TERMINATE),
            FS_READ => Some(FS_READ),
            FS_WRITE => Some(FS_WRITE),
            FS_GLOB => Some(FS_GLOB),
            METRIC_PUBLISH => Some(METRIC_PUBLISH),
            BLOB_PUT => Some(BLOB_PUT),
            BLOB_GET => Some(BLOB_GET),
            WS_SEND => Some(WS_SEND),
            WS_DIAL => Some(WS_DIAL),
            // ws/connect + ws/disconnect are INBOUND event content-type families (not effects, so not in
            // ALL) — but they're fixed kernel-defined strings, so classify them safe-to-log-verbatim too.
            WS_CONNECT => Some(WS_CONNECT),
            WS_DISCONNECT => Some(WS_DISCONNECT),
            EFFECT_REPLY => Some(EFFECT_REPLY),
            _ => None,
        }
    }

    /// The DEFAULT resolved target to PROBE a family's policy with, when building a capability manifest
    /// (host-capability-discovery I3). The manifest asks the authorizer `may this session use <family> at
    /// <probe_target>` per family — so the probe needs *some* target. These defaults are chosen (with
    /// v-agent-harness-host) to (a) read `Granted` for a BROAD grant (`permit(<family>, any-resource)`),
    /// and (b) be HARMLESS if ever dispatched (they are authorize-ONLY — never routed to an executor).
    ///
    /// - `now`/`timer`/`emit`/`shell` → `""`: ambient/any-resource families; a broad grant admits `""`. A
    ///   Prefix/exact-scoped `shell` grant reads `Denied` at `""` — honest (see the manifest semantics).
    /// - `http` → `"https://probe.invalid/"`: the RFC-6761 `.invalid` TLD is guaranteed non-resolvable, so
    ///   the probe is inert even if somehow dispatched; a broad `http` grant reads `Granted`, a `HostIn`-
    ///   scoped grant reads `Denied` at this host — honest.
    /// - `model` → `""`: there is NO session-agnostic model id (a scoped grant is `model == "<specific>"`),
    ///   so the default reads `Denied` for a scoped model grant — the session that knows its granted id
    ///   OVERRIDES the probe target (see [`super::project_manifest`]'s `probe_target` closure) for an
    ///   accurate read. The kernel default can't know it.
    ///
    /// An UNKNOWN/extension family (not in [`ALL`]) also probes with `""`.
    pub fn probe_target(family: &str) -> &'static str {
        match family {
            HTTP => "https://probe.invalid/",
            // shell/model/now/timer/emit + any extension family: no meaningful session-agnostic target.
            _ => "",
        }
    }
}

/// A `control/*` effect surfaced from the drive loop to the DRIVER (host), rather than authorized +
/// routed to an executor (control-plane partition, register-by-string design). control/* families
/// (`control/summary`, …) are host-answered, not world-actions — the kernel does NOT authorize or route
/// them; it collects them and hands them back from [`crate::kernel::Session::deliver_control`] so the
/// driver (e.g. `fork_for_query`'s watch) can consume them. `token` is the reducer's continuation token
/// (§19e); the effect's payload/family live in `request` (`request.content_type.family` is the control
/// family, e.g. `control/summary`; `request.payload` carries the summary bytes). Note `control/capabilities`
/// is the exception — it has an in-kernel handler (→ `project_manifest`), so the kernel answers it inline
/// and folds an EffectResult back instead of surfacing it here; this host-surfaced channel is for the
/// driver-consumed families like `control/summary`.
///
/// `id` is the effect's [`EffectId`]. For a FOLD-BACK control family ([`effect_ct::is_fold_back_control`],
/// e.g. `control/signature`) the drive loop gave the effect a `Dispatched` frame before surfacing it, so it
/// is OPEN and awaiting a result — the host answers off-band (e.g. reflecting the target component) and
/// settles it by `id` via [`crate::kernel::Session::settle_effect_result`], which folds the answer back to
/// the emitting reducer's continuation. For a fire-and-forget control family (`control/summary`) there is no
/// `Dispatched` frame and the `id` is not settleable (nothing to resume) — it is a stable identifier only.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ControlEffect {
    pub request: EffectRequest,
    pub token: Option<Vec<u8>>,
    pub id: EffectId,
}

impl EffectKind {
    /// The canonical lowercase family string for this kind (see [`effect_ct`]) — the string the codec
    /// writes, the router keys on, and Cedar uses as the action. One source of truth for the wire name.
    pub fn family(&self) -> &'static str {
        match self {
            EffectKind::Shell => effect_ct::SHELL,
            EffectKind::Http => effect_ct::HTTP,
            EffectKind::Model => effect_ct::MODEL,
            EffectKind::Now => effect_ct::NOW,
            EffectKind::Timer => effect_ct::TIMER,
            EffectKind::Emit => effect_ct::EMIT,
        }
    }

    /// Parse a family string back to a well-known [`EffectKind`], or `None` for an unrecognized family
    /// (a future/extension effect type that this kernel version has no built-in variant for). The inverse
    /// of [`EffectKind::family`]; the codec's `read_kind` and any string-keyed router share this mapping.
    pub fn from_family(family: &str) -> Option<EffectKind> {
        match family {
            effect_ct::SHELL => Some(EffectKind::Shell),
            effect_ct::HTTP => Some(EffectKind::Http),
            effect_ct::MODEL => Some(EffectKind::Model),
            effect_ct::NOW => Some(EffectKind::Now),
            effect_ct::TIMER => Some(EffectKind::Timer),
            effect_ct::EMIT => Some(EffectKind::Emit),
            _ => None,
        }
    }
}

/// A concrete effect the reducer wants performed: a kind plus its *resolved* target argument and
/// payload. The `target` is the SEC-F1 security-relevant string the capability predicate is checked
/// against (a URL, a repo, a command, a session id). `payload` is opaque content (large payloads are a
/// blob [`struct@Hash`]; small ones inline — §4 blob boundary), interpreted by the executor, not the kernel.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EffectRequest {
    pub kind: EffectKind,
    /// The resolved target the capability predicate gates (SEC-F1). Never trust the kind alone.
    ///
    /// `Arc<[u8]>` (operator ruling 2026-08-09): the target is UNIFORM OPAQUE BYTES, not a string — the
    /// operator's point is that the resolved targets are not all genuinely UTF-8 (a shell command, an http
    /// url, an fs path, a session id, a content hash), so modelling them as `Arc<str>` was wrong. Bytes is
    /// the honest shape. Still cheaply-clonable (an `Arc<[u8]>` clone is an O(1) refcount bump as an effect
    /// threads dispatch→authz→executor, per the cheap-clone directive). Readers that WANT a string view use
    /// [`EffectRequest::target_str`] (a fail-closed UTF-8 view); the authz gate does exactly that so a
    /// non-UTF-8 target simply fails every string predicate (fail-closed, SEC-F1-safe). `EffectRequest::new`
    /// takes `impl AsRef<[u8]>` so `&str`/`String`/`&[u8]` call sites all pass unchanged.
    pub target: std::sync::Arc<[u8]>,
    /// Opaque request body. `None` for argument-free effects (e.g. `Now`).
    pub payload: Option<Payload>,
    /// How latency-sensitive this effect is (§ operator timeliness directive). A [`Timeliness::Batchable`]
    /// effect MAY be deferred/batched by the executor for cost (e.g. Bedrock batch inference is ~half the
    /// on-demand price at higher latency); [`Timeliness::Interactive`] must run now. First-class (not a
    /// payload convention) rather than a hint smuggled through the payload — so a future executor can pick
    /// the on-demand vs batch path by reading it directly. CURRENT behavior: the field is carried on the
    /// in-memory request, but it is NOT yet recorded on the durable `Dispatched` frame ([`crate::event::EventBody`]
    /// carries kind/family/target/idempotency/deadline, not timeliness) and no executor reads it yet —
    /// wiring it through the durable frame + executor routing is a follow-up. Meaningful for `Model`; a
    /// first-class field so future batchable kinds (embeddings, bulk fetches) reuse it. Default `Interactive`.
    pub timeliness: Timeliness,
    /// The extensible content-type of this effect (seq-39): a `{family, version}` tag that routing and
    /// authz key on, so a NEW effect type is served by registering a handler for its family STRING rather
    /// than growing the [`EffectKind`] enum + recompiling the kernel. For the well-known kinds this is
    /// derived from `kind` ([`EffectKind::family`] + version 1) by [`EffectRequest::new`], so the two agree
    /// by construction; a future register-by-string slice lets an effect carry a family with no matching
    /// `EffectKind` variant at all. The `family` is the seam [`crate::authz::Authorizer`] and the executor
    /// router match on (via [`ContentType::matches_family`]).
    pub content_type: ContentType,
}

impl EffectRequest {
    /// Construct an effect request from its four fields. This is the canonical constructor — prefer it
    /// over a struct literal at every call site (kernel AND downstream crates).
    ///
    /// Why a constructor for a plain data struct: it lets the shared `EffectRequest` shape grow a field
    /// without editing every call site. Adding a field to a struct breaks every pre-existing struct literal
    /// at compile time (rustc E0063, and across a crate boundary too), so once construction goes through
    /// `new`, a field-add edits only THIS body — call sites are untouched. This is exactly how
    /// `content_type` was added: `new` DERIVES it from `kind` ([`EffectKind::family`] + version 1), so
    /// every caller that already builds via `new` got the new field for free. A caller passing a `kind`
    /// therefore always gets a matching `content_type` — the two can't drift.
    pub fn new(
        kind: EffectKind,
        target: impl AsRef<[u8]>,
        payload: Option<Payload>,
        timeliness: Timeliness,
    ) -> Self {
        EffectRequest {
            content_type: ContentType {
                // `family()` is a `&'static str` → `Cow::Borrowed`, ZERO alloc (the per-effect String this
                // used to build is exactly what the operator's Bytes/cheap-clone directive flagged).
                family: std::borrow::Cow::Borrowed(kind.family()),
                version: 1,
            },
            kind,
            // `impl AsRef<[u8]>` so `&str`/`String`/`&[u8]`/`Vec<u8>` all pass unchanged (the operator
            // Target=Bytes ruling); `Arc::from(&[u8])` is the one heap copy at construction, then O(1) clones.
            target: std::sync::Arc::from(target.as_ref()),
            payload,
            timeliness,
        }
    }

    /// Construct an effect request FROM A FAMILY STRING (effect-schema slice 2 / seq-39) — the
    /// register-by-string constructor. Where [`EffectRequest::new`] takes an [`EffectKind`] and derives the
    /// family, this takes the family directly, so a NEW effect type needs no `EffectKind` variant. The
    /// legacy `kind` field is still populated (it's not retired yet): a family with a well-known kind gets
    /// it via [`EffectKind::from_family`]; a register-by-string extension family (no built-in kind) gets the
    /// `Emit` PLACEHOLDER — kernel dispatch decisions and the idempotency key already key on the family, not
    /// the kind, so the placeholder is inert (the durable `Dispatched` frame records the family, the
    /// authoritative identity). `version` is 1. Additive alongside `new`; both yield the same shape.
    pub fn new_with_family(
        family: impl Into<std::borrow::Cow<'static, str>>,
        target: impl AsRef<[u8]>,
        payload: Option<Payload>,
        timeliness: Timeliness,
    ) -> Self {
        // Take the family as `Cow<'static, str>` (not `Arc<str>`): a well-known family passed as a
        // `&'static str` const arrives as `Cow::Borrowed` with ZERO heap alloc — the same invariant `new`
        // holds via `kind.family()` (#1563/#1722). (An `Arc<str>` parameter forced a heap alloc for every
        // `&str`/`&'static str`-const input via `Arc::from` — which is every call site here — that the match
        // below then immediately re-borrows and discards; a caller already holding an `Arc<str>` wouldn't
        // re-allocate, but none do.)
        let family: std::borrow::Cow<'static, str> = family.into();
        // Canonicalize a WELL-KNOWN family (an effect kind OR a control-plane family) to its `&'static str`
        // const → `Cow::Borrowed`, zero alloc. A well-known effect family carries its own kind; a
        // control-plane family (control/*) has no world-effect kind, so it takes the `Emit` placeholder
        // (inert — dispatch/idempotency key on family). Only a genuine register-by-string EXTENSION family
        // (unknown to both) keeps an owned string — and reuses the INPUT Cow, so a caller that already owns it
        // doesn't re-allocate.
        let (kind, family) = match EffectKind::from_family(&family) {
            Some(k) => {
                let fam = std::borrow::Cow::Borrowed(k.family());
                (k, fam)
            }
            None => match effect_ct::wellknown_control(&family) {
                Some(c) => (EffectKind::Emit, std::borrow::Cow::Borrowed(c)),
                // Extension family: keep the caller's Cow as-is (Borrowed stays borrowed, Owned isn't cloned).
                None => (EffectKind::Emit, family),
            },
        };
        EffectRequest {
            content_type: ContentType { family, version: 1 },
            kind,
            target: std::sync::Arc::from(target.as_ref()),
            payload,
            timeliness,
        }
    }

    /// A fail-closed UTF-8 STRING VIEW of the opaque byte [`target`](Self::target), for the readers that
    /// interpret it as text (a URL, an fs path, a shell command, a hex hash, a session id). `Err` when the
    /// target is not valid UTF-8 — the caller treats that as "does not match" / "not a valid <thing>",
    /// which keeps the SEC-F1 authz gate FAIL-CLOSED: a non-UTF-8 target satisfies no string predicate
    /// ([`ResourcePredicate::admits`] is fed this view, and a non-UTF-8 target is denied, never admitted).
    /// This is the ONE place the bytes→str reinterpretation lives, so every string-reading call site is a
    /// `req.target_str()` (host executors, the shell split, name resolution) rather than a scattered
    /// `str::from_utf8(&req.target)`.
    pub fn target_str(&self) -> Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(&self.target)
    }

    /// Build a STRUCTURED shell effect request (operator directive: shell invocation is a `program` + a
    /// `Vec<arg>`, NEVER a flat string whitespace-split). The `program` is the effect TARGET (the SEC-F1
    /// unit the authorizer gates — a stage program IS the gated target, matching the pipeline path); the
    /// `args` ride the PAYLOAD as a one-stage `(shell-pipeline (stage (program …) (args …)))` (see
    /// [`crate::event_ast::ShellPipeline`]), each arg a LITERAL string so an arg with spaces survives and
    /// nothing is re-split. The canonical way a reducer emits a shell command — use this instead of
    /// `EffectRequest::new(EffectKind::Shell, "cmd with args", …)` (that flat-string model, whitespace-split
    /// by the executor, is exactly what the operator's structured-args directive removes). A multi-stage
    /// pipeline uses [`crate::event_ast::encode_shell_pipeline`] directly with >1 stage.
    pub fn shell(
        program: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
        timeliness: Timeliness,
    ) -> Self {
        let program: String = program.into();
        let stage = crate::event_ast::ShellStage {
            program: program.clone(),
            args: args.into_iter().map(Into::into).collect(),
        };
        let payload = crate::event_ast::encode_shell_pipeline(&crate::event_ast::ShellPipeline {
            stages: vec![stage],
        });
        // Target = the program (the SEC-F1-gated unit); args ride the structured payload.
        EffectRequest::new(
            EffectKind::Shell,
            program.as_bytes(),
            Some(Payload::Inline(payload.into())),
            timeliness,
        )
    }
}

/// How latency-sensitive an effect is — the operator's timeliness parameter (batchable-or-not). A sum,
/// not a bool (no-sentinels standing directive): the `Batchable` arm carries an optional caller latency
/// hint, and the type grows cleanly if a third timeliness class ever appears.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum Timeliness {
    /// Latency-sensitive: the executor must run it NOW (on-demand). The default for every effect.
    #[default]
    Interactive,
    /// May be DEFERRED/batched for cost — the executor is free to route it to a cheaper, higher-latency
    /// batch path (e.g. Bedrock batch inference at ~half price). The deferred result folds back through
    /// the SAME durable dispatch→result→resume cycle whenever the batch completes (a Batchable call is
    /// just an effect whose `EffectResult` arrives much later — no new kernel machinery). `max_latency_ms`
    /// is an OPTIONAL caller hint for the longest latency it will tolerate (`None` = batch whenever); it
    /// also informs a longer auto-timeout deadline once §9d auto-timeout is wired, so a slow batch isn't
    /// prematurely cancelled.
    Batchable { max_latency_ms: Option<u64> },
}

/// An effect payload or result body: either inlined small bytes or a reference to a stored blob.
/// Keeps the log/KV thin (§4) — big transcripts/diffs live in the blob store by hash.
///
/// `Inline` holds ref-counted [`bytes::Bytes`] (operator perf directive): a payload is CLONED as it
/// crosses fold→dispatch→execute→result (and a large model-completion body is exactly the hot path), so
/// a `Bytes` clone is an O(1) refcount bump, not a deep memcpy of the whole body. Build ergonomics are
/// preserved via `Bytes: From<Vec<u8>>` — code that assembles a `Vec<u8>` freezes it with `.into()`,
/// and a reader borrows `&[u8]` (Bytes derefs to a slice), so a producer/consumer needn't care which it
/// was built from. This is what lets the executor peer (`cdz-agent-host`) keep constructing
/// `Payload::Inline(v.into())` unchanged across the flip.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Payload {
    Inline(bytes::Bytes),
    Blob(Hash),
}

/// A resource predicate: the SEC-F1 fix. A capability is not "may do `Http.get`" but "may do `Http.get`
/// to a target satisfying this predicate." Checked against the *resolved* [`EffectRequest::target`].
///
/// v0 keeps this a small, total, side-effect-free matcher (no regex-of-doom, no I/O) so authorization
/// is cheap and can never itself misbehave. It grows deliberately.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ResourcePredicate {
    /// Matches any target of this kind. Use sparingly — a broad `Http` `Any` is exactly the SSRF hole
    /// the review flagged. Prefer the scoped variants.
    Any,
    /// Target must equal this string exactly (a specific model id, a specific session).
    Exact(std::sync::Arc<str>),
    /// Target must be one of these exact strings.
    OneOf(Vec<std::sync::Arc<str>>),
    /// Target (parsed as a URL) must have a host in this allow-list. The SSRF/exfil guard for `Http`.
    HostIn(Vec<std::sync::Arc<str>>),
    /// Target must start with this prefix (e.g. a command allow-list, a path/repo scope).
    Prefix(std::sync::Arc<str>),
    /// Target (a SessionId = genesis hash) must be a transitive `Spawned`-DESCENDANT of `controller`
    /// (§lifecycle supervision-tree authority, I6). This is a DECLARATIVE MARKER only: the kernel cannot
    /// compute the descendant set here — [`admits`](Self::admits) has no access to the session registry
    /// (it's a per-session, replay-deterministic check called during `deliver` while the registry is
    /// borrowed; walking the live spawn tree at authorize-time would also be §4b replay-unsafe). So this
    /// arm always [`admits`] FALSE (fail-closed); the HOST enforces the real relation by FREEZING the
    /// controller's transitive descendant-set into a concrete predicate (a [`OneOf`](Self::OneOf) of the
    /// descendant SessionIds) at `set_authorizer` time — it has the registry + re-bakes on each new
    /// `Spawned` edge, so the frozen set is a replay-safe snapshot. The `controller` hash rides the wire
    /// (manifest codec) as its hex so a discovery reader can see WHICH controller a `lifecycle/*` grant is
    /// scoped under, even though the kernel's own `admits` never green-lights it. Locked shape with
    /// v-agent-harness-host (option-a): kernel = inert marker + wire vocab, host = the tree-walk.
    DescendantOf(Hash),
}

impl ResourcePredicate {
    /// Does `target` satisfy this predicate? Total, pure, cheap. This is the SEC-F1 enforcement point.
    pub fn admits(&self, target: &str) -> bool {
        match self {
            ResourcePredicate::Any => true,
            ResourcePredicate::Exact(s) => target == s.as_ref(),
            ResourcePredicate::OneOf(set) => set.iter().any(|s| s.as_ref() == target),
            ResourcePredicate::HostIn(hosts) => match host_of(target) {
                // Host comparison is case- and trailing-dot-insensitive (RFC 3986 §3.2.2: host is
                // case-insensitive; `ok.host.` is the same host as `ok.host`). Exact-string `==` here
                // was a bug: it wrongly DENIED `OK.host`/`ok.host.` (fail-closed, but a real
                // correctness gap). Still fail-closed — normalization only ever makes the SAME host
                // match, never widens to a different one.
                Some(h) => hosts.iter().any(|allowed| host_eq(allowed, &h)),
                None => false, // unparseable target → deny (fail closed)
            },
            ResourcePredicate::Prefix(p) => target.starts_with(p.as_ref()),
            // A DECLARATIVE MARKER (I6): the kernel has no registry here to walk the spawn tree, so this
            // fails closed — the host re-bakes it into a concrete OneOf(descendant-set) at set_authorizer
            // time (see the variant doc). A `DescendantOf` grant that reaches `admits` unfrozen denies.
            ResourcePredicate::DescendantOf(_) => false,
        }
    }
}

/// A single grant: this effect *kind*, restricted to targets satisfying `predicate` (SEC-F1).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Capability {
    pub kind: EffectKind,
    pub predicate: ResourcePredicate,
}

impl Capability {
    /// Does this grant permit the given request? The effect FAMILY must match AND the predicate must admit
    /// the resolved target. Both conditions — the review's whole point (SEC-F1): family alone is not enough.
    ///
    /// Family-keyed (seq-39): the match is `req.content_type.family == self.kind.family()`, via
    /// [`ContentType::matches_family`], NOT an `EffectKind` enum equality. So authz keys on the same family
    /// STRING the codec/router use — the seam that lets a future effect type (a family with no built-in
    /// `EffectKind`) be granted by family without a kernel enum edit. For the well-known kinds this is
    /// identical to the old `kind ==` check (a request built via [`EffectRequest::new`] has
    /// `content_type.family == kind.family()` by construction).
    pub fn permits(&self, req: &EffectRequest) -> bool {
        // Feed the predicate the fail-closed UTF-8 view of the opaque byte target (operator Target=Bytes
        // ruling): a non-UTF-8 target admits nothing (SEC-F1 stays fail-closed — a malformed target is
        // never granted).
        req.content_type.matches_family(self.kind.family())
            && req.target_str().is_ok_and(|t| self.predicate.admits(t))
    }

    /// Grant an effect FAMILY that has no built-in [`EffectKind`] — the register-by-string authz seam
    /// (seq-39). A [`Capability`] can only name a family via `kind.family()`, i.e. one of the six well-known
    /// kinds; a `store/*` family (§4c) — or any future register-by-string family — has NO `EffectKind`, so it
    /// is otherwise UNGRANTABLE. This returns a [`FamilyGrant`] naming the family STRING directly; hand it to
    /// [`crate::authz::Authorizer::with_family_grants`] (or the host's grant set) to permit that family under `predicate`.
    ///
    /// For `store/*` the predicate gates the NAME (the effect target): e.g.
    /// `Capability::for_family(effect_ct::STORE_SET, ResourcePredicate::Prefix("system/".into()))` permits
    /// `store/set` on any `system/…` name — the §4c write-authority grant. Additive: it does NOT change the
    /// `Capability` struct (its 45 kernel+host literals are untouched) — a NEW grant shape alongside it.
    pub fn for_family(
        family: impl Into<std::sync::Arc<str>>,
        predicate: ResourcePredicate,
    ) -> FamilyGrant {
        FamilyGrant {
            family: family.into(),
            predicate,
        }
    }
}

/// A grant keyed by an effect FAMILY STRING (not an [`EffectKind`]) — the register-by-string authz grant
/// (see [`Capability::for_family`]). Permits a request whose `content_type.family` equals `family` AND
/// whose resolved target satisfies `predicate` (SEC-F1, same two-condition rule as [`Capability::permits`]).
/// The grant shape for families with no built-in kind — `store/*` (§4c) today, any extension family later.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FamilyGrant {
    /// The exact effect family this grant names (e.g. `"store/set"`).
    pub family: std::sync::Arc<str>,
    /// The resource predicate the request's resolved target must satisfy (SEC-F1). For `store/*` this gates
    /// the NAME (e.g. `Prefix("system/")` → may set any `system/…` name).
    pub predicate: ResourcePredicate,
}

impl FamilyGrant {
    /// Does this family-grant permit `req`? Family STRING match AND predicate admits the resolved target —
    /// the same SEC-F1 two-condition rule [`Capability::permits`] applies, keyed on the family string.
    pub fn permits(&self, req: &EffectRequest) -> bool {
        // Fail-closed UTF-8 view (operator Target=Bytes ruling): a non-UTF-8 target satisfies no predicate.
        req.content_type.matches_family(&self.family)
            && req.target_str().is_ok_and(|t| self.predicate.admits(t))
    }
}

/// Whether a session may use a given effect family — one entry of a [`CapabilityManifest`]. Computed from
/// the TWO actual sources (host mechanism + policy), never a hand-maintained list, so it can't drift:
/// `Absent` when the host has no executor for the family; otherwise `Granted`/`Denied` by the authorizer's
/// decision. (The design leaves room for a future `Requestable` split of `Denied` — host has the executor
/// but policy denies AND the session may request it — once that policy model is ratified; today a
/// policy-denied family is a single `Denied`.)
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum GrantState {
    /// The host has an executor for this family AND policy admits this session's probe → usable now.
    Granted,
    /// The host has an executor for this family, but policy denied the probe.
    Denied,
    /// The host has NO executor for this family — unusable regardless of policy.
    Absent,
}

/// One family's entry in the capability manifest: the family string, its grant-state, and (when known) the
/// resource scope the grant is bound to. `scope` REUSES [`ResourcePredicate`] (never a new scope type); it
/// is `None` when there is no scope to report (an `Absent` family, or a `Denied` one).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CapabilityEntry {
    /// The effect family this entry describes. `Arc<str>` (operator cheaply-clonable directive: an id/name/
    /// family is a cheaply-clonable `Arc<str>`, never an owned `String`) — a well-known family is a
    /// `&'static str` const so `.into()` is an `Arc::from(&str)` once at build, then O(1) refcount clones as
    /// the manifest threads through projection/diff/answer.
    pub family: std::sync::Arc<str>,
    pub grant: GrantState,
    pub scope: Option<ResourcePredicate>,
}

/// A session's capability manifest (§host-capability-discovery I1): one [`CapabilityEntry`] per well-known
/// effect family. The reducer learns "what effect families may I use, and at what scope" from this — the
/// authorized projection of host mechanism ∩ policy. Content-typed as `capabilities-manifest` when it rides
/// the log (a later slice); this is the in-kernel value.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct CapabilityManifest {
    pub entries: Vec<CapabilityEntry>,
}

/// One family's grant-state transition between two manifests — the unit of a capability CHANGE (the
/// §host-capability-discovery I6 reactive-push input). `from`/`to` are the [`GrantState`] before and after
/// (they differ — an unchanged family produces no [`GrantChange`]); `family` names which effect family moved.
/// A push consumer reads this to decide relevance (a change touching a family this session could use) and to
/// carry what moved (e.g. `Absent → Granted` = a new executor/grant landed; `Granted → Denied` = a policy
/// tightened). Scope-only changes are deliberately NOT reported here — I6 wakes on grant-STATE moves, and the
/// snapshot manifest carries the current scope; a scope-only refinement doesn't change usability.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GrantChange {
    /// Which effect family's grant-state moved. `Arc<str>` (operator cheaply-clonable directive) — a
    /// cheaply-clonable family handle, never an owned `String`; sourced from the manifest entry's `family`
    /// (itself `Arc<str>`), so producing a change is an O(1) refcount clone.
    pub family: std::sync::Arc<str>,
    pub from: GrantState,
    pub to: GrantState,
}

impl CapabilityManifest {
    /// The per-family grant-state DELTA from `prev` to `self` — the pure heart of the I6 reactive push:
    /// "which families' usability changed, and how." Empty iff no family's [`GrantState`] moved (so a
    /// session whose projected manifest didn't change gets NO `capabilities-changed` push — the design's
    /// "delivered ONLY to sessions whose projected manifest actually changed"). A family present in one
    /// manifest but not the other is treated as a move to/from [`GrantState::Absent`] (an absent entry and
    /// an `Absent`-state entry mean the same thing to a consumer: unusable). Deterministic order: by family
    /// string, so the delta (and any log frame built from it) is replay-stable.
    ///
    /// Compares grant STATE only, not scope — see [`GrantChange`]. Builds a family→state index for each
    /// manifest (one pass each), then walks the union of family names once — an O(n log n) pass (the
    /// `BTreeMap` also gives the replay-stable family-string order for free), not a find-per-family scan.
    pub fn grant_changes(&self, prev: &CapabilityManifest) -> Vec<GrantChange> {
        use std::collections::BTreeMap;
        // Index each manifest family→state in one pass (a BTreeMap keyed on family — its ordered keys are the
        // replay-stable delta order, so no separate sort). A family absent from a map reads as `Absent`.
        let self_idx: BTreeMap<&str, &GrantState> = self
            .entries
            .iter()
            .map(|e| (e.family.as_ref(), &e.grant))
            .collect();
        let prev_idx: BTreeMap<&str, &GrantState> = prev
            .entries
            .iter()
            .map(|e| (e.family.as_ref(), &e.grant))
            .collect();
        // Walk the union of family names (BTreeMap keys are sorted → the walk is ordered).
        self_idx
            .keys()
            .chain(prev_idx.keys())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .filter_map(|&fam| {
                let from = prev_idx
                    .get(fam)
                    .copied()
                    .cloned()
                    .unwrap_or(GrantState::Absent);
                let to = self_idx
                    .get(fam)
                    .copied()
                    .cloned()
                    .unwrap_or(GrantState::Absent);
                if from == to {
                    return None;
                }
                Some(GrantChange {
                    family: fam.into(),
                    from,
                    to,
                })
            })
            .collect()
    }
}

/// Project a session's capability manifest by PROBING (the LOCKED crux — not authorizer enumeration).
/// For each family in the canonical set (`families`, normally [`effect_ct::ALL`]): the mechanism dimension
/// is `handles(family)` (does the host have an executor?), and the policy dimension is ONE `authorize`
/// probe (the existing decide-only [`crate::authz::Authorize`] trait — no enumeration API). Complete BY CONSTRUCTION: the
/// family set is finite + canonical, so nothing is missed. Pure: deterministic given the inputs; async only
/// because the authorizer may `.await` a wasm policy eval.
///
/// **`probe_target` is per-family** (`Fn(&str) -> &str`): the resolved target to probe each family's policy
/// with — normally [`effect_ct::probe_target`] (the kernel default, one source of truth), which a
/// host/session OVERRIDES for a family whose grant is target-scoped (esp. `model`, whose scoped grant is a
/// specific id no generic probe matches). The probed request is built via [`EffectRequest::new`] so its
/// `content_type.family` matches the family being probed.
///
/// **Grant-state semantics (decide-only, "grantable-at-probe-target"):** a decide-only authorizer
/// fundamentally cannot report a SCOPED grant's admissibility without the real target — so the manifest is
/// honest about what it probed:
/// - [`GrantState::Absent`] — the host has NO executor for the family (`handles` false); unusable
///   regardless of policy. This is the mechanism axis, distinct from a policy denial.
/// - [`GrantState::Granted`] — mechanism present AND policy admits the probe target: usable at (at least)
///   the probe target. A broad grant admits all targets; a scoped grant admits at least this one.
/// - [`GrantState::Denied`] — mechanism present but policy denied the probe target. This does NOT mean
///   "never usable": a scoped grant (e.g. `model == "<id>"`, a `HostIn` host, a `Prefix`) may well admit the
///   session's REAL target — the reducer discovers the exact decision when it emits a concrete effect
///   (override `probe_target` for that family to get an accurate read here). Absent (no executor) is the
///   distinct, always-actionable signal; Denied-at-probe is "maybe, at your target — emit to find out."
pub async fn project_manifest(
    families: &[&str],
    handles: impl Fn(&str) -> bool,
    authorizer: &(impl crate::authz::Authorize + ?Sized),
    probe_target: impl Fn(&str) -> &'static str,
) -> CapabilityManifest {
    let mut entries = Vec::with_capacity(families.len());
    for &family in families {
        let grant = if !handles(family) {
            GrantState::Absent
        } else {
            // Mechanism present → the policy probe decides. Build the probe request via the family's
            // well-known kind when there is one (so kind + content_type.family agree); an extension family
            // with no `EffectKind` still probes by family once register-by-string lands.
            let kind = EffectKind::from_family(family).unwrap_or(EffectKind::Emit);
            let mut probe =
                EffectRequest::new(kind, probe_target(family), None, Timeliness::Interactive);
            probe.content_type.family = family.to_string().into();
            match authorizer.authorize(&probe).await {
                Ok(()) => GrantState::Granted,
                Err(_) => GrantState::Denied,
            }
        };
        entries.push(CapabilityEntry {
            family: family.into(),
            grant,
            scope: None,
        });
    }
    CapabilityManifest { entries }
}

/// Extract the host from a `scheme://host[:port]/…` target, for `HostIn`. Uses the battle-tested `url`
/// crate (RFC 3986 / WHATWG) rather than a hand-rolled authority parser — operator directive: a bespoke
/// parser guarding an ALLOW-LIST is the worst place for edge-case bugs, since every missed case is an
/// authz BYPASS (Copilot PR#1015/1018 were exactly IPv6-literal bypasses in the old hand-rolled code).
/// `url` handles IPv6 literals, userinfo, ports, and normalization correctly, so this just parses and
/// pulls `.host_str()`.
///
/// Fail-closed (SEC-F1): any parse error, or a URL with no host authority (`mailto:`, a relative/opaque
/// target, an empty host), yields `None` → deny. Returns an OWNED `String` because `host_str()` borrows
/// from the parsed `Url` (a local). For an IPv6 literal `url` gives the bracketed form (`[::1]`); we
/// strip the brackets so the returned host is the bare address (`::1`) — what a `HostIn` entry carries.
fn host_of(target: &str) -> Option<String> {
    let parsed = url::Url::parse(target).ok()?;
    let host = parsed.host_str()?;
    if host.is_empty() {
        return None;
    }
    // IPv6 literals come back bracketed (`[::1]`); a HostIn allow-list entry is the bare address. Strip
    // exactly one matching pair. (A reg-name / IPv4 never has brackets, so this is a no-op for them.)
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    Some(host.to_string())
}

/// Host equality for `HostIn` (RFC 3986 §3.2.2): ASCII-case-insensitive and insensitive to a single
/// trailing dot (the FQDN root, `ok.host.` ≡ `ok.host`). Pure + total; only ever matches the SAME
/// host, so it can't widen a capability to a different target.
fn host_eq(allowed: &str, actual: &str) -> bool {
    let norm = |h: &str| h.strip_suffix('.').unwrap_or(h).to_ascii_lowercase();
    norm(allowed) == norm(actual)
}

#[cfg(test)]
mod tests {
    use super::*;

    // #2180 residual tracing redaction: only EXACT well-known families resolve to a fixed &'static (safe to
    // log); an extension family — or a guest string that merely SHARES a control/store PREFIX — returns None
    // so the logger redacts it (a prefix like `store/<secret>` must NOT be treated as safe).
    #[test]
    fn wellknown_static_str_matches_exact_families_and_rejects_extensions() {
        use effect_ct::*;
        assert_eq!(wellknown_static_str(HTTP), Some(HTTP));
        assert_eq!(wellknown_static_str(SHELL), Some(SHELL));
        assert_eq!(wellknown_static_str(EMIT), Some(EMIT));
        assert_eq!(wellknown_static_str(CAPABILITIES), Some(CAPABILITIES));
        assert_eq!(wellknown_static_str(STORE_SET), Some(STORE_SET));
        assert_eq!(wellknown_static_str(STORE_ADD), Some(STORE_ADD));
        assert_eq!(wellknown_static_str(STORE_REMOVE), Some(STORE_REMOVE));
        assert_eq!(
            wellknown_static_str(STORE_RESOLVE_ALL),
            Some(STORE_RESOLVE_ALL)
        );
        assert_eq!(wellknown_static_str(FS_READ), Some(FS_READ));
        assert_eq!(wellknown_static_str(FS_WRITE), Some(FS_WRITE));
        assert_eq!(wellknown_static_str(FS_GLOB), Some(FS_GLOB));
        assert_eq!(wellknown_static_str(METRIC_PUBLISH), Some(METRIC_PUBLISH));
        assert_eq!(wellknown_static_str(BLOB_PUT), Some(BLOB_PUT));
        assert_eq!(wellknown_static_str(BLOB_GET), Some(BLOB_GET));
        assert_eq!(wellknown_static_str(WS_SEND), Some(WS_SEND));
        assert_eq!(wellknown_static_str(WS_DIAL), Some(WS_DIAL));
        // Extension families (register-by-string, guest-controlled) → None (redacted).
        assert_eq!(wellknown_static_str("my/custom-effect"), None);
        assert_eq!(wellknown_static_str("weather"), None);
        // A guest string sharing a control/store PREFIX is STILL guest bytes → None (prefix ≠ safe).
        assert_eq!(wellknown_static_str("store/secret-token-abc123"), None);
        assert_eq!(wellknown_static_str("control/leak-me"), None);
    }

    // Authz-exempt set (userspace-effects D2): control families (host-answered) AND `effect/reply` (its
    // target is an opaque non-UTF-8 reply-token a capability predicate can't admit; the host ReplyExecutor
    // does the stronger cryptographic token check). Every OTHER family — built-in or extension — goes
    // through the SEC-F1 capability gate. Exemption is authz-only, not a routing change.
    #[test]
    fn is_authz_exempt_covers_control_families_and_effect_reply_only() {
        use effect_ct::*;
        // effect/reply is exempt (token-authorized by the host executor, not capability-gated).
        assert!(is_authz_exempt(EFFECT_REPLY));
        // Control families are exempt (host-answered, never capability-gated).
        assert!(is_authz_exempt(CAPABILITIES));
        assert!(is_authz_exempt("control/anything"));
        // Ordinary executor-routed effects are NOT exempt — they take the SEC-F1 gate.
        assert!(!is_authz_exempt(HTTP));
        assert!(!is_authz_exempt(SHELL));
        assert!(!is_authz_exempt(STORE_SET));
        assert!(!is_authz_exempt("weather")); // an extension family
                                              // A guest string merely sharing the `effect/` prefix (a registry pointer name, not the reply verb)
                                              // is NOT exempt — only the exact `effect/reply` family is.
        assert!(!is_authz_exempt("effect/weather"));
    }

    // §4c session-directory I3: the store sub-partition split — both pointer and group verbs share the
    // `store/` prefix (both authz-gated on the name), but the drive loop routes GROUP verbs to
    // apply_group_effect (member-op payload) and pointer verbs to apply_effect (name-set payload).
    #[test]
    fn is_group_store_family_splits_group_verbs_from_pointer_verbs() {
        use effect_ct::*;
        // Group OR-set verbs → true.
        assert!(is_group_store_family(STORE_ADD));
        assert!(is_group_store_family(STORE_REMOVE));
        assert!(is_group_store_family(STORE_RESOLVE_ALL));
        // Single-value pointer verbs → false (they're store/*, but NOT group).
        assert!(!is_group_store_family(STORE_SET));
        assert!(!is_group_store_family(STORE_RESOLVE));
        // …yet every group verb is still in the store partition (shares the prefix + authz gate).
        assert!(is_store_family(STORE_ADD));
        assert!(is_store_family(STORE_REMOVE));
        assert!(is_store_family(STORE_RESOLVE_ALL));
        // A guest string sharing the prefix is NOT a known group verb (exact-match, not prefix).
        assert!(!is_group_store_family("store/add-evil"));
        assert!(!is_group_store_family("http"));
    }

    fn http(target: &str) -> EffectRequest {
        // Exercises the canonical constructor (the effect-schema-arc migration path).
        EffectRequest::new(EffectKind::Http, target, None, Timeliness::Interactive)
    }

    #[test]
    fn new_with_family_derives_kind_for_wellknown_and_placeholders_extensions() {
        // effect-schema slice 2: the register-by-string constructor. A WELL-KNOWN family derives its kind
        // (and equals the enum constructor's shape); an EXTENSION family (no built-in kind) gets the Emit
        // placeholder while preserving the real family — the durable identity dispatch/idempotency key on.
        let http = EffectRequest::new_with_family(
            effect_ct::HTTP,
            "https://ok/x",
            None,
            Timeliness::Interactive,
        );
        assert_eq!(http.content_type.family, effect_ct::HTTP);
        assert_eq!(
            http.kind,
            EffectKind::Http,
            "well-known family derives its kind"
        );
        // Same shape as the enum constructor for a well-known family.
        let via_enum = EffectRequest::new(
            EffectKind::Http,
            "https://ok/x",
            None,
            Timeliness::Interactive,
        );
        assert_eq!(http.content_type, via_enum.content_type);
        assert_eq!(http.kind, via_enum.kind);

        // A well-known family is a zero-alloc Cow::Borrowed (the #1563/#1722 invariant).
        assert!(
            matches!(http.content_type.family, std::borrow::Cow::Borrowed(_)),
            "a well-known family is Cow::Borrowed (zero-alloc)"
        );

        // A well-known CONTROL family (no EffectKind) is ALSO zero-alloc Borrowed, via wellknown_control —
        // the #1727 residual: it used to fall to None→Owned. Emit placeholder, family preserved.
        let caps = EffectRequest::new_with_family(
            effect_ct::CAPABILITIES,
            "self",
            None,
            Timeliness::Interactive,
        );
        assert_eq!(caps.content_type.family, effect_ct::CAPABILITIES);
        assert_eq!(caps.kind, EffectKind::Emit);
        assert!(
            matches!(caps.content_type.family, std::borrow::Cow::Borrowed(_)),
            "a well-known control family is Cow::Borrowed (zero-alloc), not owned"
        );

        // An extension family with no EffectKind variant → Emit placeholder, family preserved. Passed as a
        // `&'static str`, it stays Cow::Borrowed — ZERO alloc even for an unknown family (the #1722 fix: the
        // constructor takes `Into<Cow<'static, str>>` and preserves the caller's Cow instead of round-tripping
        // through an Arc<str>, so a static extension family no longer allocates either).
        let ext =
            EffectRequest::new_with_family("custom/metrics", "m", None, Timeliness::Interactive);
        assert_eq!(ext.content_type.family, "custom/metrics");
        assert_eq!(
            ext.kind,
            EffectKind::Emit,
            "an extension family with no built-in kind gets the Emit placeholder"
        );
        assert!(
            matches!(ext.content_type.family, std::borrow::Cow::Borrowed(_)),
            "a static extension family stays Borrowed (zero-alloc — the #1722 fix)"
        );

        // A genuinely OWNED extension family (a runtime String, not a static const) is preserved as
        // Cow::Owned WITHOUT re-allocation — the constructor reuses the caller's Cow, it doesn't clone it.
        let dynamic: String = format!("custom/{}", "runtime");
        let owned_ext = EffectRequest::new_with_family(
            std::borrow::Cow::Owned(dynamic),
            "m",
            None,
            Timeliness::Interactive,
        );
        assert_eq!(owned_ext.content_type.family, "custom/runtime");
        assert!(
            matches!(owned_ext.content_type.family, std::borrow::Cow::Owned(_)),
            "a runtime-owned extension family is preserved as Owned (reused, not re-cloned)"
        );
    }

    #[test]
    fn a_store_set_request_carries_the_family_not_the_placeholder_kind_for_authz() {
        // The request-layer BACKSTOP for #1916 (ComponentAuthorizer authorizes on content_type.family, NOT
        // the EffectKind enum): a register-by-string store/* request carries the REAL family string while its
        // `kind` is the inert `Emit` PLACEHOLDER. This pins the PRECONDITION that fix relies on — if this
        // invariant ever broke (e.g. store/set started carrying kind=Shell, or the family got lost), the
        // policy would gate the wrong string. The end-to-end decision test lives in the host's Cedar e2e
        // (a forbid on action=="store/set" denies a store/set); this is the cheap kernel-side 2nd layer
        // catching a regression at the request-construction boundary, no policy fixture needed.
        let set = EffectRequest::new_with_family(
            effect_ct::STORE_SET,
            "system/compiler/latest",
            None,
            Timeliness::Interactive,
        );
        assert_eq!(
            set.content_type.family, effect_ct::STORE_SET,
            "a store/set request carries family 'store/set' — the string authz (and a Cedar policy) sees"
        );
        assert_eq!(
            set.kind,
            EffectKind::Emit,
            "store/* has no EffectKind variant → the inert Emit placeholder; authz MUST key on family, not \
             this kind (else store/set authorizes as 'emit' — the #1916 bug)"
        );
        // The family and the placeholder kind's family DIFFER — the exact gap that makes keying-on-kind wrong.
        assert_ne!(
            set.content_type.family.as_ref(),
            EffectKind::Emit.family(),
            "the store/set family must NOT equal the placeholder kind's family, or the bug would be invisible"
        );
    }

    #[test]
    fn new_derives_content_type_from_kind_and_matches_a_full_literal() {
        // EffectRequest::new DERIVES content_type from kind (family = kind.family(), version 1) — this is
        // the field-add benefit realized: callers pass the same 4 args and get the extra field filled
        // consistently, so kind and content_type.family can't drift. Equivalent to a full literal that
        // spells out the derived content_type.
        let via_new = EffectRequest::new(
            EffectKind::Http,
            "https://ok.host/x",
            Some(Payload::Inline(b"body".to_vec().into())),
            Timeliness::Interactive,
        );
        // new() set content_type.family to the kind's canonical family string.
        assert_eq!(via_new.content_type.family, EffectKind::Http.family());
        assert_eq!(via_new.content_type.version, 1);
        let via_literal = EffectRequest {
            kind: EffectKind::Http,
            target: "https://ok.host/x".as_bytes().into(),
            payload: Some(Payload::Inline(b"body".to_vec().into())),
            timeliness: Timeliness::Interactive,
            content_type: ContentType {
                family: EffectKind::Http.family().into(),
                version: 1,
            },
        };
        assert_eq!(via_new, via_literal);
        // The `impl Into<Arc<str>>` target arg accepts both &str and String uniformly.
        assert_eq!(
            EffectRequest::new(
                EffectKind::Now,
                String::new(),
                None,
                Timeliness::Interactive
            ),
            EffectRequest::new(EffectKind::Now, "", None, Timeliness::Interactive),
        );
    }

    #[test]
    fn shell_builds_a_structured_program_plus_args_request_no_whitespace_split() {
        // operator directive: shell invocation is a structured {program, args}, NEVER a flat string split.
        // EffectRequest::shell puts the PROGRAM in the target (the SEC-F1-gated unit) + the args in a
        // one-stage (shell-pipeline …) payload — each arg literal, so an arg WITH SPACES survives intact.
        let req = EffectRequest::shell(
            "echo",
            ["hello world", ";", "not-a-separator"],
            Timeliness::Interactive,
        );
        assert_eq!(req.kind, EffectKind::Shell);
        // Target = the program (what authz gates), NOT the whole command line.
        assert_eq!(req.target_str().unwrap(), "echo");
        // Args ride the structured payload; decode it back and confirm they are LITERAL + un-split.
        let Some(Payload::Inline(bytes)) = &req.payload else {
            panic!("shell request must carry a structured pipeline payload");
        };
        let pipeline = crate::event_ast::decode_shell_pipeline(bytes).expect("decodes");
        assert_eq!(
            pipeline.stages.len(),
            1,
            "a single command is a one-stage pipeline"
        );
        let stage = &pipeline.stages[0];
        assert_eq!(stage.program, "echo");
        assert_eq!(
            stage.args,
            vec![
                "hello world".to_string(), // an arg WITH A SPACE survives as ONE arg (the whole point)
                ";".to_string(), // a metacharacter is a literal arg, never a shell separator
                "not-a-separator".to_string(),
            ]
        );
    }

    #[test]
    fn control_family_partition_keys_on_the_control_prefix_and_effect_families_stay_bare() {
        // The control-plane partition (register-by-string design): control/* families are authz-exempt +
        // host-answered; effect families are bare + executor-routed. is_control_family is the one-source
        // prefix test the drive loop applies before authorize/route.
        assert!(effect_ct::is_control_family(effect_ct::CAPABILITIES));
        assert!(effect_ct::is_control_family(effect_ct::SUMMARY));
        assert!(effect_ct::is_control_family(effect_ct::SIGNATURE));
        assert!(effect_ct::is_control_family("control/anything"));
        // Every well-known EFFECT family stays BARE — NOT in the control namespace (durable-wire constraint:
        // they can't gain an "effect/" prefix, and must never be misclassified as control).
        for &fam in effect_ct::ALL {
            assert!(
                !effect_ct::is_control_family(fam),
                "effect family {fam:?} must NOT be control"
            );
        }
        // The control consts actually carry the prefix (byte-for-byte), and CONTROL_PREFIX is "control/".
        assert_eq!(effect_ct::CONTROL_PREFIX, "control/");
        assert!(effect_ct::CAPABILITIES.starts_with(effect_ct::CONTROL_PREFIX));
        assert!(effect_ct::SUMMARY.starts_with(effect_ct::CONTROL_PREFIX));
        assert!(effect_ct::SIGNATURE.starts_with(effect_ct::CONTROL_PREFIX));
        // control/signature is a well-known control family: exact string, safe-logging, and wellknown_control
        // canonicalizes it (zero-alloc Borrowed) — the composable-component-calls signature-query (v0).
        assert_eq!(effect_ct::SIGNATURE, "control/signature");
        assert_eq!(
            effect_ct::wellknown_control(effect_ct::SIGNATURE),
            Some(effect_ct::SIGNATURE)
        );
        assert_eq!(
            effect_ct::wellknown_static_str(effect_ct::SIGNATURE),
            Some(effect_ct::SIGNATURE)
        );
        // A guest control/<secret> still redacts (prefix ≠ safe), unchanged.
        assert_eq!(effect_ct::wellknown_static_str("control/secret-x"), None);
        // A bare family that merely CONTAINS "control" but doesn't start with the prefix is NOT control.
        assert!(!effect_ct::is_control_family("my-control-thing"));
        assert!(!effect_ct::is_control_family(""));
    }

    #[test]
    fn store_family_partition_is_distinct_from_control_and_effect_families() {
        // §4c store/* partition: store/set + store/resolve are AUTHZ-GATED store effects (the write layer),
        // a THIRD partition alongside control/* (authz-exempt) and bare effect families (executor-routed).
        assert!(effect_ct::is_store_family(effect_ct::STORE_SET));
        assert!(effect_ct::is_store_family(effect_ct::STORE_RESOLVE));
        assert!(effect_ct::is_store_family("store/anything"));
        assert_eq!(effect_ct::STORE_PREFIX, "store/");
        assert!(effect_ct::STORE_SET.starts_with(effect_ct::STORE_PREFIX));
        assert!(effect_ct::STORE_RESOLVE.starts_with(effect_ct::STORE_PREFIX));
        // store/* is NOT control (it IS gated) and control/* is NOT store — the partitions are disjoint.
        assert!(!effect_ct::is_control_family(effect_ct::STORE_SET));
        assert!(!effect_ct::is_store_family(effect_ct::CAPABILITIES));
        // Bare effect families are neither store nor control.
        for &fam in effect_ct::ALL {
            assert!(!effect_ct::is_store_family(fam), "effect {fam:?} not store");
        }
        // A family merely CONTAINING "store" but not prefixed is not a store family.
        assert!(!effect_ct::is_store_family("my-store"));
        assert!(!effect_ct::is_store_family(""));
    }

    #[test]
    fn lifecycle_family_partition_consts_predicate_manifest_and_safe_logging() {
        // §lifecycle session-control partition: lifecycle/{spawn,suspend,resume,terminate} — a NEW
        // authz-gated partition (executor-routed, NOT kernel-handled like store/control).
        assert_eq!(effect_ct::LIFECYCLE_PREFIX, "lifecycle/");
        for f in [
            effect_ct::LIFECYCLE_SPAWN,
            effect_ct::LIFECYCLE_SUSPEND,
            effect_ct::LIFECYCLE_RESUME,
            effect_ct::LIFECYCLE_TERMINATE,
        ] {
            assert!(f.starts_with(effect_ct::LIFECYCLE_PREFIX));
            assert!(
                effect_ct::is_lifecycle_family(f),
                "{f:?} is a lifecycle family"
            );
            // SAFE-LOGGING (#2180): the exact lifecycle strings are kernel-defined fixed &'static, so they
            // log VERBATIM (not redacted-to-length) — wellknown_static_str returns Some for each.
            assert_eq!(effect_ct::wellknown_static_str(f), Some(f));
            // They're in the manifest family set → the capability projection reports lifecycle grant-states.
            assert!(
                effect_ct::ALL.contains(&f),
                "{f:?} is projected in the capability manifest"
            );
        }
        assert_eq!(effect_ct::LIFECYCLE_SPAWN, "lifecycle/spawn");
        assert_eq!(effect_ct::LIFECYCLE_TERMINATE, "lifecycle/terminate");
        // DISJOINT from store/control: lifecycle is neither (it's authz-gated + executor-routed).
        assert!(!effect_ct::is_store_family(effect_ct::LIFECYCLE_SPAWN));
        assert!(!effect_ct::is_control_family(effect_ct::LIFECYCLE_SPAWN));
        assert!(!effect_ct::is_lifecycle_family(effect_ct::STORE_SET));
        assert!(!effect_ct::is_lifecycle_family(effect_ct::CAPABILITIES));
        // A guest-controlled extension family under a fake lifecycle-ish name that ISN'T the exact const
        // logs redacted (None), not verbatim — the #2180 prefix-isn't-enough rule.
        assert_eq!(effect_ct::wellknown_static_str("lifecycle/secret-x"), None);
        // But the prefix predicate still classifies it (routing/authz partition is prefix-based).
        assert!(effect_ct::is_lifecycle_family("lifecycle/secret-x"));
        assert!(!effect_ct::is_lifecycle_family("my-lifecycle"));
    }

    #[test]
    fn fs_family_partition_consts_predicate_manifest_and_safe_logging() {
        // §GAP-3 filesystem partition: fs/{read,write,glob} — a NEW authz-gated partition (executor-routed
        // to the host FsExecutor, NOT kernel-handled like store/control), gated on the resolved PATH target.
        assert_eq!(effect_ct::FS_PREFIX, "fs/");
        for f in [effect_ct::FS_READ, effect_ct::FS_WRITE, effect_ct::FS_GLOB] {
            assert!(f.starts_with(effect_ct::FS_PREFIX));
            assert!(effect_ct::is_fs_family(f), "{f:?} is an fs family");
            // SAFE-LOGGING (#2180): the exact fs strings are kernel-defined fixed &'static → log VERBATIM.
            assert_eq!(effect_ct::wellknown_static_str(f), Some(f));
            // In the manifest family set → the capability projection reports fs grant-states.
            assert!(
                effect_ct::ALL.contains(&f),
                "{f:?} is projected in the capability manifest"
            );
        }
        assert_eq!(effect_ct::FS_READ, "fs/read");
        assert_eq!(effect_ct::FS_WRITE, "fs/write");
        assert_eq!(effect_ct::FS_GLOB, "fs/glob");
        // DISJOINT from the other partitions.
        assert!(!effect_ct::is_store_family(effect_ct::FS_READ));
        assert!(!effect_ct::is_control_family(effect_ct::FS_READ));
        assert!(!effect_ct::is_lifecycle_family(effect_ct::FS_READ));
        assert!(!effect_ct::is_fs_family(effect_ct::STORE_SET));
        assert!(!effect_ct::is_fs_family(effect_ct::LIFECYCLE_SPAWN));
        // A guest-controlled extension family under a fake fs-ish name that ISN'T an exact const logs
        // redacted (None), not verbatim (#2180 prefix-isn't-enough); the prefix predicate still classifies it.
        assert_eq!(effect_ct::wellknown_static_str("fs/secret-x"), None);
        assert!(effect_ct::is_fs_family("fs/secret-x"));
        assert!(!effect_ct::is_fs_family("myfs"));
    }

    #[test]
    fn metric_family_partition_consts_predicate_manifest_and_safe_logging() {
        // §operator-Q3 metrics-publish partition: metric/publish — authz-gated, executor-routed to the host
        // MetricExecutor, register-by-string (no new EffectKind), gated on the metric NAME target.
        assert_eq!(effect_ct::METRIC_PREFIX, "metric/");
        assert_eq!(effect_ct::METRIC_PUBLISH, "metric/publish");
        assert!(effect_ct::is_metric_family(effect_ct::METRIC_PUBLISH));
        assert!(effect_ct::METRIC_PUBLISH.starts_with(effect_ct::METRIC_PREFIX));
        // SAFE-LOGGING (#2180): the exact string is kernel-defined fixed &'static → log VERBATIM.
        assert_eq!(
            effect_ct::wellknown_static_str(effect_ct::METRIC_PUBLISH),
            Some(effect_ct::METRIC_PUBLISH)
        );
        // In the manifest family set → the capability projection reports metric grant-states.
        assert!(effect_ct::ALL.contains(&effect_ct::METRIC_PUBLISH));
        // DISJOINT from the other partitions.
        assert!(!effect_ct::is_store_family(effect_ct::METRIC_PUBLISH));
        assert!(!effect_ct::is_fs_family(effect_ct::METRIC_PUBLISH));
        assert!(!effect_ct::is_lifecycle_family(effect_ct::METRIC_PUBLISH));
        assert!(!effect_ct::is_metric_family(effect_ct::FS_READ));
        // A guest fake metric-ish name that isn't the exact const logs redacted (None); prefix still classifies.
        assert_eq!(effect_ct::wellknown_static_str("metric/secret-x"), None);
        assert!(effect_ct::is_metric_family("metric/secret-x"));
        assert!(!effect_ct::is_metric_family("mymetric"));
    }

    #[test]
    fn blob_family_partition_consts_predicate_manifest_and_safe_logging() {
        // cadenza-docs I3 blob CAS-write partition: blob/put + blob/get — authz-gated, executor-routed to the
        // host blob executor over BlobStore, register-by-string (no new EffectKind). The reducer-facing effect
        // that invokes the blob.rs put/get storage primitive (a reducer emits effects; it can't call put directly).
        assert_eq!(effect_ct::BLOB_PREFIX, "blob/");
        assert_eq!(effect_ct::BLOB_PUT, "blob/put");
        assert_eq!(effect_ct::BLOB_GET, "blob/get");
        assert!(effect_ct::is_blob_family(effect_ct::BLOB_PUT));
        assert!(effect_ct::is_blob_family(effect_ct::BLOB_GET));
        assert!(effect_ct::BLOB_PUT.starts_with(effect_ct::BLOB_PREFIX));
        // SAFE-LOGGING (#2180): the exact strings are kernel-defined fixed &'static → log VERBATIM.
        assert_eq!(
            effect_ct::wellknown_static_str(effect_ct::BLOB_PUT),
            Some(effect_ct::BLOB_PUT)
        );
        assert_eq!(
            effect_ct::wellknown_static_str(effect_ct::BLOB_GET),
            Some(effect_ct::BLOB_GET)
        );
        // In the manifest family set → the capability projection reports blob grant-states.
        assert!(effect_ct::ALL.contains(&effect_ct::BLOB_PUT));
        assert!(effect_ct::ALL.contains(&effect_ct::BLOB_GET));
        // DISJOINT from the other partitions (blob/* is neither store nor fs — a distinct content-addressed store).
        assert!(!effect_ct::is_store_family(effect_ct::BLOB_PUT));
        assert!(!effect_ct::is_fs_family(effect_ct::BLOB_PUT));
        assert!(!effect_ct::is_metric_family(effect_ct::BLOB_PUT));
        assert!(!effect_ct::is_lifecycle_family(effect_ct::BLOB_PUT));
        assert!(!effect_ct::is_blob_family(effect_ct::FS_WRITE));
        assert!(!effect_ct::is_blob_family(effect_ct::STORE_SET));
        // A guest fake blob-ish name that isn't the exact const logs redacted (None); prefix still classifies.
        assert_eq!(effect_ct::wellknown_static_str("blob/secret-x"), None);
        assert!(effect_ct::is_blob_family("blob/secret-x"));
        assert!(!effect_ct::is_blob_family("myblob"));
    }

    #[test]
    fn ws_family_is_a_routed_partition_disjoint_from_the_others() {
        // THE OUTPOST O1: ws/* is the reducer OUTBOUND-websocket partition. Executor-routed + authz-gated
        // (register-by-string, no new EffectKind), like fs/*/blob/*/metric/*. O1 ships ws/send; the WS_PREFIX
        // reserves the namespace so a later ws/close slots in additively.
        assert_eq!(effect_ct::WS_PREFIX, "ws/");
        assert_eq!(effect_ct::WS_SEND, "ws/send");
        assert!(effect_ct::is_ws_family(effect_ct::WS_SEND));
        assert!(effect_ct::WS_SEND.starts_with(effect_ct::WS_PREFIX));
        // ws/dial — the OUTBOUND hub-federation effect (F0-effect), mirrors ws/send: in the ws/* family,
        // safe-to-log verbatim, and a grantable capability in ALL (unlike the inbound ws/connect events).
        assert_eq!(effect_ct::WS_DIAL, "ws/dial");
        assert!(effect_ct::is_ws_family(effect_ct::WS_DIAL));
        assert_eq!(
            effect_ct::wellknown_static_str(effect_ct::WS_DIAL),
            Some(effect_ct::WS_DIAL)
        );
        assert!(effect_ct::ALL.contains(&effect_ct::WS_DIAL));
        // SAFE-LOGGING (#2180): the exact string is a kernel-defined fixed &'static → log VERBATIM.
        assert_eq!(
            effect_ct::wellknown_static_str(effect_ct::WS_SEND),
            Some(effect_ct::WS_SEND)
        );
        // In the manifest family set → the capability projection reports the ws grant-state. Only the
        // OUTBOUND EFFECT (ws/send) is a grantable capability; the inbound EVENT families are NOT in ALL.
        assert!(effect_ct::ALL.contains(&effect_ct::WS_SEND));
        // DISJOINT from every other partition (a distinct outbound-transport family).
        assert!(!effect_ct::is_store_family(effect_ct::WS_SEND));
        assert!(!effect_ct::is_fs_family(effect_ct::WS_SEND));
        assert!(!effect_ct::is_metric_family(effect_ct::WS_SEND));
        assert!(!effect_ct::is_blob_family(effect_ct::WS_SEND));
        assert!(!effect_ct::is_lifecycle_family(effect_ct::WS_SEND));
        assert!(!effect_ct::is_control_family(effect_ct::WS_SEND));
        assert!(!effect_ct::is_ws_family(effect_ct::FS_WRITE));
        // A guest fake ws-ish name that isn't the exact const logs redacted (None); prefix still classifies.
        assert_eq!(effect_ct::wellknown_static_str("ws/secret-x"), None);
        assert!(effect_ct::is_ws_family("ws/secret-x"));
        assert!(!effect_ct::is_ws_family("myws"));

        // ws/connect + ws/disconnect (operator #2804): INBOUND EVENT content-type families the host emits on
        // peer connect/close. In the ws/* namespace + safe-logging classified (fixed kernel strings), but
        // NOT effects → NOT in ALL (they're not grantable capabilities, they're events the reducer folds).
        assert_eq!(effect_ct::WS_CONNECT, "ws/connect");
        assert_eq!(effect_ct::WS_DISCONNECT, "ws/disconnect");
        assert!(effect_ct::is_ws_family(effect_ct::WS_CONNECT));
        assert!(effect_ct::is_ws_family(effect_ct::WS_DISCONNECT));
        assert_eq!(
            effect_ct::wellknown_static_str(effect_ct::WS_CONNECT),
            Some(effect_ct::WS_CONNECT)
        );
        assert_eq!(
            effect_ct::wellknown_static_str(effect_ct::WS_DISCONNECT),
            Some(effect_ct::WS_DISCONNECT)
        );
        // The inbound event families are NOT grantable effects → absent from ALL (only ws/send is).
        assert!(!effect_ct::ALL.contains(&effect_ct::WS_CONNECT));
        assert!(!effect_ct::ALL.contains(&effect_ct::WS_DISCONNECT));
    }

    #[test]
    fn effect_reply_is_a_routed_grantable_family_not_a_userspace_registration_candidate() {
        // userspace-effects I4: effect/reply is the ROUTED OUTBOUND family a handler emits to answer a
        // forwarded request (target = reply-token, payload = response). It shares the `effect/` prefix with
        // EFFECT_REGISTRY_PREFIX but is a BUILT-IN routed effect, NOT a userspace-registration target.
        assert_eq!(effect_ct::EFFECT_REPLY, "effect/reply");
        assert!(effect_ct::EFFECT_REPLY.starts_with(effect_ct::EFFECT_REGISTRY_PREFIX));
        // Grantable (in ALL — the capability projection reports it) + safe-logging (fixed &'static).
        assert!(effect_ct::ALL.contains(&effect_ct::EFFECT_REPLY));
        assert_eq!(
            effect_ct::wellknown_static_str(effect_ct::EFFECT_REPLY),
            Some(effect_ct::EFFECT_REPLY)
        );
        // CRITICAL collision guard: despite the effect/ prefix, effect/reply is NOT a userspace-effect
        // family — it must never route to handler resolution (it IS the reply verb, a built-in).
        assert!(
            !effect_ct::is_registered_effect_family(effect_ct::EFFECT_REPLY),
            "effect/reply is a built-in routed family, NOT a userspace-registration candidate"
        );
        // A genuine userspace family (no effect/ prefix, not a builtin) still IS a candidate.
        assert!(effect_ct::is_registered_effect_family("weather"));
        // A guest effect/<x> that isn't the exact reply const logs redacted (None); the registry PREFIX is
        // a store-name space, not an effect-family — so wellknown_static_str doesn't bless arbitrary effect/*.
        assert_eq!(effect_ct::wellknown_static_str("effect/weather"), None);
    }

    #[test]
    fn effect_kind_family_round_trips_and_matches_the_canonical_consts() {
        // Every well-known kind maps to its canonical family string and back — the vocab the codec,
        // router, and Cedar action-map all share (extensible-effects seam, seq-39).
        for kind in [
            EffectKind::Shell,
            EffectKind::Http,
            EffectKind::Model,
            EffectKind::Now,
            EffectKind::Timer,
            EffectKind::Emit,
        ] {
            assert_eq!(EffectKind::from_family(kind.family()), Some(kind.clone()));
        }
        // The consts are the exact lowercase names (byte-for-byte — the codec/authz depend on these).
        assert_eq!(EffectKind::Http.family(), effect_ct::HTTP);
        assert_eq!(EffectKind::Http.family(), "http");
        // An unrecognized family (a future/extension effect type) → None, not a panic.
        assert_eq!(EffectKind::from_family("summary"), None);
        assert_eq!(EffectKind::from_family("not-a-kind"), None);
    }

    #[test]
    fn effect_family_strings_are_a_stable_lowercase_ascii_wire_contract() {
        // Slice 1 made the `effect_ct` family strings a CROSS-SITE WIRE CONTRACT: they are the exact
        // bytes the codec writes into the durable append-only log, the key the executor router matches
        // on, and the Cedar policy ACTION name. So their literal values are frozen — a rename (even one
        // that keeps family()/from_family consistent, e.g. "shell"→"Shell") would silently break on-disk
        // log compatibility + every deployed Cedar policy. The round-trip test only pins http's literal;
        // this pins the WHOLE vocab byte-for-byte, plus the two structural invariants a string-keyed
        // registry needs (no collisions, canonical lowercase-ascii) so a future extension can't drift.
        let vocab = [
            (EffectKind::Shell, effect_ct::SHELL, "shell"),
            (EffectKind::Http, effect_ct::HTTP, "http"),
            (EffectKind::Model, effect_ct::MODEL, "model"),
            (EffectKind::Now, effect_ct::NOW, "now"),
            (EffectKind::Timer, effect_ct::TIMER, "timer"),
            (EffectKind::Emit, effect_ct::EMIT, "emit"),
        ];
        // 1. Every const is its exact frozen literal, and family() returns that same const (one source).
        for (kind, konst, literal) in &vocab {
            assert_eq!(
                konst, literal,
                "the {literal:?} family const drifted from its wire byte"
            );
            assert_eq!(
                &kind.family(),
                konst,
                "family() must return the effect_ct const verbatim"
            );
        }
        // 2. The family strings are pairwise UNIQUE — a string-keyed router/authz would be ambiguous if
        //    two kinds shared a name (a copy-paste typo like MODEL:"http"), so pin no-collision directly.
        for i in 0..vocab.len() {
            for j in (i + 1)..vocab.len() {
                assert_ne!(
                    vocab[i].1, vocab[j].1,
                    "family strings must be unique: {} and {} collide",
                    vocab[i].1, vocab[j].1
                );
            }
        }
        // 3. Canonical form: lowercase ASCII, non-empty (the doc's "lowercase FAMILY string" promise —
        //    a new extension const that broke this would route/authorize inconsistently across sites).
        for (_, konst, _) in &vocab {
            assert!(!konst.is_empty(), "a family string must be non-empty");
            assert!(
                konst.chars().all(|c| c.is_ascii_lowercase()),
                "family string {konst:?} must be lowercase ascii (canonical wire form)"
            );
        }
    }

    #[test]
    fn timeliness_defaults_to_interactive_and_batchable_carries_its_hint() {
        // Default is Interactive (the operator's latency-sensitive default — every effect runs now
        // unless it opts into batching).
        assert_eq!(Timeliness::default(), Timeliness::Interactive);
        // A Batchable request carries its optional caller latency hint (a sum, not a bool — the hint
        // rides the variant). None = batch whenever; Some(ms) = the longest latency tolerated.
        let deferred = EffectRequest::new(
            EffectKind::Model,
            "anthropic.claude",
            None,
            Timeliness::Batchable {
                max_latency_ms: Some(3_600_000),
            },
        );
        match deferred.timeliness {
            Timeliness::Batchable { max_latency_ms } => assert_eq!(max_latency_ms, Some(3_600_000)),
            Timeliness::Interactive => panic!("expected Batchable"),
        }
        // Batchable-whenever (no hint) is distinct from Interactive.
        assert_ne!(
            Timeliness::Batchable {
                max_latency_ms: None
            },
            Timeliness::Interactive
        );
    }

    #[test]
    fn host_allow_list_blocks_imds_and_exfil() {
        let cap = Capability {
            kind: EffectKind::Http,
            predicate: ResourcePredicate::HostIn(vec!["metrics.internal".into()]),
        };
        assert!(cap.permits(&http("https://metrics.internal/v1/query")));
        // The SEC-F1 attacks: same kind, hostile target — must be denied.
        assert!(!cap.permits(&http("http://169.254.169.254/latest/meta-data/")));
        assert!(!cap.permits(&http("https://attacker.example/exfil?d=secret")));
    }

    #[test]
    fn kind_mismatch_is_denied_even_if_predicate_matches() {
        let cap = Capability {
            kind: EffectKind::Shell,
            predicate: ResourcePredicate::Any,
        };
        assert!(!cap.permits(&http("https://anything")));
    }

    #[test]
    fn unparseable_url_fails_closed() {
        let pred = ResourcePredicate::HostIn(vec!["ok.host".into()]);
        assert!(!pred.admits("not a url"));
        assert!(!pred.admits("https://"));
    }

    #[test]
    fn host_parsing_strips_port_and_userinfo() {
        assert_eq!(
            host_of("https://user:pw@host.tld:8443/path").as_deref(),
            Some("host.tld")
        );
        assert_eq!(host_of("http://h.tld").as_deref(), Some("h.tld"));
    }

    #[test]
    fn host_parsing_handles_ipv6_literals() {
        // Bracketed IPv6 host must not be split on its internal colons (the old code returned "[").
        assert_eq!(host_of("http://[::1]/latest").as_deref(), Some("::1"));
        assert_eq!(
            host_of("https://[2001:db8::1]:8443/x").as_deref(),
            Some("2001:db8::1")
        );
        // And an IPv6 allow-list entry now actually matches its target.
        let pred = ResourcePredicate::HostIn(vec!["::1".into()]);
        assert!(pred.admits("http://[::1]/latest"));
        assert!(!pred.admits("http://[2001:db8::2]/x"));
    }

    #[test]
    fn ipv6_bracket_with_trailing_junk_is_denied_not_bypassed() {
        // Copilot PR#1015 (SEC-F1 allow-list BYPASS): after the closing `]` the only valid tail is empty
        // or `:port`. `[::1]evil.com` must NOT parse as host `::1` (which a HostIn(["::1"]) grant would
        // then AUTHORIZE, reaching evil.com). Fail closed: unparseable → None → deny.
        assert_eq!(host_of("http://[::1]evil.com/"), None);
        assert_eq!(host_of("http://[::1]x/"), None);
        let pred = ResourcePredicate::HostIn(vec!["::1".into()]);
        assert!(
            !pred.admits("http://[::1]evil.com/"),
            "an ::1 grant must NOT authorize [::1]evil.com — that's the bypass"
        );
        // The legitimate forms still parse (guard against over-rejecting).
        assert_eq!(host_of("http://[::1]:8080/").as_deref(), Some("::1"));
        assert_eq!(host_of("http://[::1]/").as_deref(), Some("::1"));
    }

    #[test]
    fn ipv6_bracket_tail_must_be_empty_or_a_real_numeric_port() {
        // Copilot PR#1018 REGRESSION repro (the bypass that mattered): a HOSTILE colon-tail after `]`
        // must NOT parse as host `::1` — else a HostIn(["::1"]) grant would authorize the hostile
        // target. The `url` crate correctly REJECTS all of these (malformed authority → parse error →
        // None → deny), so the bypass stays closed under the new parser:
        let pred = ResourcePredicate::HostIn(vec!["::1".into()]);
        assert_eq!(host_of("http://[::1]:80evil.com/"), None, ":80evil.com");
        assert_eq!(host_of("http://[::1]:0x50/"), None, "non-decimal port");
        assert_eq!(host_of("http://[::1]:80.evil/"), None, "port then junk");
        assert!(
            !pred.admits("http://[::1]:80evil.com/"),
            "an ::1 grant must NOT authorize [::1]:80evil.com — the regression bypass"
        );
        // NOTE a real difference from the old hand-rolled parser: `[::1]:` (bracket + a bare, EMPTY port)
        // is a VALID authority per WHATWG/RFC-3986 — the host genuinely IS `::1` (trailing empty port =
        // default port), NOT a hostile tail. The old parser wrongly REJECTED it (over-strict, fail-closed
        // but a false denial); `url` accepts it as host `::1`, which is correct + not a bypass. So this
        // assertion is UPDATED (the old `None` expectation was the hand-rolled parser's bug, not a
        // security property):
        assert_eq!(
            host_of("http://[::1]:/").as_deref(),
            Some("::1"),
            "empty port is valid, host is ::1"
        );
        // Real numeric ports still parse to the bare host.
        assert_eq!(host_of("http://[::1]:80/").as_deref(), Some("::1"));
        assert_eq!(
            host_of("http://[2001:db8::1]:443/x").as_deref(),
            Some("2001:db8::1")
        );
    }

    #[test]
    fn host_match_is_case_and_trailing_dot_insensitive() {
        // RFC 3986 §3.2.2: host is case-insensitive, and a trailing-dot FQDN is the same host. The old
        // exact `==` wrongly DENIED these (fail-closed, but a real correctness bug that breaks a
        // legitimately-granted request).
        let pred = ResourcePredicate::HostIn(vec!["ok.host".into()]);
        assert!(pred.admits("https://OK.Host/x"), "case-insensitive");
        assert!(pred.admits("https://ok.host./x"), "trailing-dot FQDN");
        assert!(pred.admits("https://ok.HOST.:443/x"), "both");
        // Normalization must NOT widen to a DIFFERENT host (still fail-closed on real mismatches).
        assert!(!pred.admits("https://ok.host.evil.com/x"));
        assert!(!pred.admits("https://notok.host/x"));
    }

    #[test]
    fn prefix_scopes_commands() {
        let cap = Capability {
            kind: EffectKind::Shell,
            predicate: ResourcePredicate::Prefix("cargo ".into()),
        };
        let ok = EffectRequest::new(
            EffectKind::Shell,
            "cargo test",
            None,
            Timeliness::Interactive,
        );
        let bad = EffectRequest::new(EffectKind::Shell, "rm -rf /", None, Timeliness::Interactive);
        assert!(cap.permits(&ok));
        assert!(!cap.permits(&bad));
    }

    #[test]
    fn descendant_of_is_an_inert_marker_that_admits_nothing_in_the_kernel() {
        // I6 supervision-tree authority: the kernel's admits() has no registry to walk the spawn tree, so a
        // DescendantOf predicate FAILS CLOSED here — it admits NO target, whatever the controller or target.
        // (The host re-bakes it into a concrete OneOf(descendant-set) at set_authorizer time; unfrozen, it
        // must never green-light a lifecycle/* effect — that's the fail-closed safety of the marker.)
        let controller = Hash::of(b"controller-session");
        let pred = ResourcePredicate::DescendantOf(controller);
        // Neither the controller itself, nor an arbitrary would-be descendant, nor an empty target is admitted.
        assert!(
            !pred.admits(&controller.to_hex()),
            "not even the controller itself"
        );
        assert!(!pred.admits(&Hash::of(b"some-child").to_hex()));
        assert!(!pred.admits(""));
        // A lifecycle/* family-grant carrying an unfrozen DescendantOf therefore permits nothing.
        let grant = Capability::for_family(effect_ct::LIFECYCLE_TERMINATE, pred);
        let req = EffectRequest::new_with_family(
            effect_ct::LIFECYCLE_TERMINATE,
            Hash::of(b"some-child").to_hex(),
            None,
            Timeliness::Interactive,
        );
        assert!(
            !grant.permits(&req),
            "an unfrozen DescendantOf grant admits no lifecycle target in the kernel"
        );
    }

    // ---- host-capability-discovery I1: manifest projection by probing --------------------------------

    #[tokio::test(flavor = "current_thread")]
    async fn project_manifest_computes_the_three_grant_states_from_mechanism_and_policy() {
        use crate::authz::Authorizer;

        // Policy: grant Http (any target) only. Mechanism (handles): the host serves Http + Model, but NOT
        // Shell. So over {http, model, shell} the projection must yield exactly one of each grant-state:
        //  - http  → GRANTED  (mechanism yes + policy allows)
        //  - model → DENIED   (mechanism yes + policy denies — the REQUESTABLE-precursor state)
        //  - shell → ABSENT   (mechanism no — policy irrelevant)
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Http,
            predicate: ResourcePredicate::Any,
        }]);
        let handles = |f: &str| f == effect_ct::HTTP || f == effect_ct::MODEL;
        let families = [effect_ct::HTTP, effect_ct::MODEL, effect_ct::SHELL];

        let manifest = project_manifest(&families, handles, &authz, |_| "probe://scope").await;

        assert_eq!(manifest.entries.len(), 3);
        let state = |fam: &str| {
            manifest
                .entries
                .iter()
                .find(|e| e.family.as_ref() == fam)
                .map(|e| e.grant.clone())
                .unwrap()
        };
        assert_eq!(state(effect_ct::HTTP), GrantState::Granted);
        assert_eq!(state(effect_ct::MODEL), GrantState::Denied);
        assert_eq!(state(effect_ct::SHELL), GrantState::Absent);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn project_manifest_over_all_is_complete_by_construction() {
        use crate::authz::Authorizer;
        // Probing the canonical `effect_ct::ALL` set yields exactly one entry per known family — nothing
        // missed (the crux: the family set is finite + canonical). With deny_all + no mechanism, every
        // family is Absent (handles=false short-circuits before the policy probe).
        let manifest =
            project_manifest(effect_ct::ALL, |_| false, &Authorizer::deny_all(), |_| "x").await;
        assert_eq!(manifest.entries.len(), effect_ct::ALL.len());
        assert!(manifest
            .entries
            .iter()
            .all(|e| e.grant == GrantState::Absent));
        // Every canonical family is represented.
        for &fam in effect_ct::ALL {
            assert!(manifest.entries.iter().any(|e| e.family.as_ref() == fam));
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn project_manifest_driven_by_the_real_composite_executor_mechanism_source() {
        // I2: the projection's `handles` closure fed by the REAL CompositeExecutor::handles_family accessor
        // (not a test stub) — the actual wiring the host uses. Register a Now executor only; over ALL, the
        // registered family reflects the policy decision (Granted here — deny-nothing grant), every other
        // family is Absent (no executor). Proves handles_family is the mechanism source project_manifest
        // consumes.
        use crate::authz::Authorizer;
        use crate::executor::{CompositeExecutor, RecordingExecutor};

        let exec = CompositeExecutor::new()
            .with_effect(effect_ct::NOW, Box::new(RecordingExecutor::new()));
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Now,
            predicate: ResourcePredicate::Any,
        }]);
        let manifest = project_manifest(
            effect_ct::ALL,
            |f| exec.handles_family(f),
            &authz,
            effect_ct::probe_target,
        )
        .await;

        for entry in &manifest.entries {
            if entry.family.as_ref() == effect_ct::NOW {
                assert_eq!(entry.grant, GrantState::Granted, "Now: mechanism + policy");
            } else {
                assert_eq!(
                    entry.grant,
                    GrantState::Absent,
                    "{}: no executor",
                    entry.family
                );
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn probe_target_default_reads_scoped_grants_as_denied_and_override_reads_them_granted() {
        // I3: the "grantable-at-probe-target" semantics + the host-override path (v-ah-host decision (2)).
        // A SCOPED model grant (model == "claude-x") — the host serves model, policy grants only that id.
        use crate::authz::Authorizer;
        use crate::executor::{CompositeExecutor, RecordingExecutor};
        let exec = CompositeExecutor::new()
            .with_effect(effect_ct::MODEL, Box::new(RecordingExecutor::new()));
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Model,
            predicate: ResourcePredicate::Exact("claude-x".into()),
        }]);
        let model_state = |m: &CapabilityManifest| {
            m.entries
                .iter()
                .find(|e| e.family.as_ref() == effect_ct::MODEL)
                .unwrap()
                .grant
                .clone()
        };

        // Default probe_target(model) == "" ≠ "claude-x" → DENIED-at-probe (honest: mechanism present, but
        // policy denies the generic probe target). NOT Absent — the executor IS registered.
        let with_default = project_manifest(
            effect_ct::ALL,
            |f| exec.handles_family(f),
            &authz,
            effect_ct::probe_target,
        )
        .await;
        assert_eq!(model_state(&with_default), GrantState::Denied);

        // The session that KNOWS its granted id OVERRIDES the probe target for model → GRANTED (accurate).
        let over = |family: &str| {
            if family == effect_ct::MODEL {
                "claude-x"
            } else {
                effect_ct::probe_target(family)
            }
        };
        let with_override =
            project_manifest(effect_ct::ALL, |f| exec.handles_family(f), &authz, over).await;
        assert_eq!(model_state(&with_override), GrantState::Granted);

        // effect_ct::probe_target defaults: http gets the .invalid probe, the rest empty.
        assert_eq!(
            effect_ct::probe_target(effect_ct::HTTP),
            "https://probe.invalid/"
        );
        assert_eq!(effect_ct::probe_target(effect_ct::MODEL), "");
        assert_eq!(effect_ct::probe_target("some-extension-family"), "");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn manifest_over_all_with_a_realistic_mixed_grant_and_the_real_probe_defaults() {
        // End-to-end: the WHOLE canonical set, the REAL CompositeExecutor mechanism source, the REAL
        // effect_ct::probe_target defaults, and a realistic MIXED grant — the integration behavior a host
        // gets. Grants: http scoped to the probe host (reads Granted at the default probe), now broad
        // (Granted), model scoped to a specific id (Denied at the "" default — honest), shell/timer/emit no
        // executor (Absent). Proves the http probe default ("https://probe.invalid/") distinguishes a
        // matching HostIn grant (Granted) from the empty/absent cases, not just Any grants.
        use crate::authz::Authorizer;
        use crate::executor::{CompositeExecutor, RecordingExecutor};

        // Host serves http + now + model; NOT shell/timer/emit.
        let exec = CompositeExecutor::new()
            .with_effect(effect_ct::HTTP, Box::new(RecordingExecutor::new()))
            .with_effect(effect_ct::NOW, Box::new(RecordingExecutor::new()))
            .with_effect(effect_ct::MODEL, Box::new(RecordingExecutor::new()));
        let authz = Authorizer::new(vec![
            // http granted exactly at the default probe host → Granted at the real probe target.
            Capability {
                kind: EffectKind::Http,
                predicate: ResourcePredicate::HostIn(vec!["probe.invalid".into()]),
            },
            // now broad → Granted.
            Capability {
                kind: EffectKind::Now,
                predicate: ResourcePredicate::Any,
            },
            // model scoped to a specific id → Denied at the "" default probe (honest; override to read it).
            Capability {
                kind: EffectKind::Model,
                predicate: ResourcePredicate::Exact("claude-x".into()),
            },
        ]);

        let m = project_manifest(
            effect_ct::ALL,
            |f| exec.handles_family(f),
            &authz,
            effect_ct::probe_target,
        )
        .await;
        let g = |fam: &str| {
            m.entries
                .iter()
                .find(|e| e.family.as_ref() == fam)
                .unwrap()
                .grant
                .clone()
        };
        assert_eq!(
            g(effect_ct::HTTP),
            GrantState::Granted,
            "http: HostIn matches the probe host"
        );
        assert_eq!(g(effect_ct::NOW), GrantState::Granted, "now: broad grant");
        assert_eq!(
            g(effect_ct::MODEL),
            GrantState::Denied,
            "model: scoped, denied at the \"\" probe"
        );
        assert_eq!(
            g(effect_ct::SHELL),
            GrantState::Absent,
            "shell: no executor"
        );
        assert_eq!(
            g(effect_ct::TIMER),
            GrantState::Absent,
            "timer: no executor"
        );
        assert_eq!(g(effect_ct::EMIT), GrantState::Absent, "emit: no executor");
    }

    // ---- host-capability-discovery I6: manifest grant-state delta (reactive-push input) --------------

    #[test]
    fn grant_changes_reports_only_moved_families_in_stable_order() {
        // I6's pure heart: which families' usability CHANGED between two projected manifests, and how — the
        // input to the `capabilities-changed` push (delivered ONLY to sessions whose manifest actually moved).
        let entry = |family: &str, grant: GrantState| CapabilityEntry {
            family: family.into(),
            grant,
            scope: None,
        };
        // prev: http Granted, model Denied, shell Absent.
        let prev = CapabilityManifest {
            entries: vec![
                entry(effect_ct::HTTP, GrantState::Granted),
                entry(effect_ct::MODEL, GrantState::Denied),
                entry(effect_ct::SHELL, GrantState::Absent),
            ],
        };
        // now: model policy loosened (Denied→Granted), shell executor registered (Absent→Granted), http
        // unchanged. Entries listed out of family order to prove the delta is sorted, not input-ordered.
        let now = CapabilityManifest {
            entries: vec![
                entry(effect_ct::SHELL, GrantState::Granted),
                entry(effect_ct::HTTP, GrantState::Granted),
                entry(effect_ct::MODEL, GrantState::Granted),
            ],
        };
        let changes = now.grant_changes(&prev);
        assert_eq!(
            changes,
            vec![
                GrantChange {
                    family: effect_ct::MODEL.into(),
                    from: GrantState::Denied,
                    to: GrantState::Granted,
                },
                GrantChange {
                    family: effect_ct::SHELL.into(),
                    from: GrantState::Absent,
                    to: GrantState::Granted,
                },
            ],
            "only model + shell moved, in family-string order (http unchanged → omitted)"
        );

        // No move → empty delta (the "session's manifest didn't change → no push" gate).
        assert!(
            now.grant_changes(&now).is_empty(),
            "an unchanged manifest yields no grant changes"
        );

        // A family present in only ONE manifest is a move to/from Absent (absent entry ≡ Absent state).
        let with_timer = CapabilityManifest {
            entries: vec![entry(effect_ct::TIMER, GrantState::Granted)],
        };
        let empty = CapabilityManifest::default();
        assert_eq!(
            with_timer.grant_changes(&empty),
            vec![GrantChange {
                family: effect_ct::TIMER.into(),
                from: GrantState::Absent,
                to: GrantState::Granted,
            }],
            "a newly-present Granted family reads as Absent→Granted vs an empty manifest"
        );
        // ...and the reverse direction (losing a family) is Granted→Absent, symmetric.
        assert_eq!(
            empty.grant_changes(&with_timer),
            vec![GrantChange {
                family: effect_ct::TIMER.into(),
                from: GrantState::Granted,
                to: GrantState::Absent,
            }],
            "losing a family reads as Granted→Absent"
        );
    }
}
