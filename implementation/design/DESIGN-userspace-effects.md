# Userspace-defined effects — handler sessions registered in the name server (platform extension + capability encapsulation)

Owner: TBD (a `vertical`; kernel-facing slices are `v-agent-harness`'s zone, host-facing slices
`v-agent-harness-host`'s — see "Ownership & build sequencing"). Design by `design-userspace-effects`.
Status: **PROPOSAL — shaped autonomously WITH the harness owners, awaiting operator ratification of the
forks + a build owner.** Operator spark via concierge 2026-08-08 (verbatim): *"What would be super
cool in the kernel is to allow for userspace defined effects that get registered in the name server.
This would allow the whole platform to be extended. And actually… this is how we do the encapsulation
of permissions right? So we would be able to install a privileged effect handler that gets requests
from other sessions and could scope down the level of permissions, kind of like the date example. It
would have arbitrary shell access but the interface would only allow for a date read and could
register its own resources to do authz on. And I guess the effect handler is really just another
session/reducer, right? Like it would have its own state and interface, etc."* The operator is on
Slack and could not run a live design session; this proposal was shaped with the peers below. Forks
the operator may want to own on return are flagged **⟨operator may ratify⟩**, each with a chosen
default so the build is not blocked.

Subsystem: `cdz-kernel` (the one new durable seam — deferred effect settlement — plus the `effect/*`
registration namespace + resolution) and `cdz-agent-host` (the delegating executor + the reply path +
the handler-session driver). Coordinated with `v-agent-harness` (owns the session model, `effect_ct`
family vocabulary, `name_store` §4c, `SessionId = genesis-hash-hex`), `v-agent-harness-host` (owns
`EmitExecutor` / `AsyncAgentHost` loop / shared `Inbox` / the Cedar `ComponentAuthorizer` §20b),
`design-dogwood-cedar` (the authz resource/entity model — I5, DEFERRED to the operator's return),
`design-cadenza-docs` (the binary-AST doc model for introspection I11), and `v-effects` (effect
semantics). **THE OUTPOST collapses INTO this model** (an outpost is a userspace effect handler, not a
host capability — see the capstone below); this IS the minimize-kernel theme's fullest expression.

> **Vocabulary caution (from `v-effects`, who owns the language-level meaning of "effect"/"resume").**
> This document is about KERNEL-family effects: family-string routing (`effect_ct`), a caller that
> SUSPENDS on an `EffectId` and RESUMES on the settling `EffectResult` at the host boundary. That is an
> ordinary routed-effect round-trip and does NOT touch the rcdzc language-level algebraic-effects fold
> (handle / perform / resume as a compile-time handler-arm continuation, abortive-vs-resumptive). Keep
> the two vocabularies visibly distinct: language-level *handler arm / resume / abortive* (v-effects's
> lane) vs kernel *EffectId / EffectResult / settle* (this doc's lane). A reader from the guide must not
> conflate them. v-effects confirmed no shape overlap; offered a fold-surface sign-off once landed.

> **This is the stateful generalization of a DECIDED design, not a new axis.** §20a of
> `design/agent-harness-kernel.md` (RESOURCE-RESCOPING components, "Rust unsafe-block for
> capabilities", *decided*) already ratifies the core idea: a published component INTERNALLY holds a
> broad capability (`shell(target="date")`) but EXPORTS only a narrow virtual resource
> (`date.now → string`), so N callers hold only the cheap narrow grant and the dangerous grant lives
> with ONE audited program. The operator's spark GENERALIZES that from a stateless, per-invoke
> *component* into a stateful, long-lived *handler session* — "just another session/reducer" with its
> own KV state, its own log, its own interface, that fields requests from other sessions over time. So
> this design does not introduce a new capability model; it makes the §20a rescoping unit be a
> **session** and gives it a **registration + request-routing + reply** protocol built entirely from
> existing seams. Read §20a and §12f (attenuating delegation) before building — this is their runtime
> realization, not a competitor to them.

## The organizing principle (operator capstone, 2026-08-08)

> Operator, verbatim: *"anything stateful/kernel side looks like any other effect handler in the
> system."*

Everything in this document is one idea applied repeatedly: **the kernel/host owns only minimal,
stateful, transport-level byte-movers; everything application-level is a userspace handler session.**
A handler session is spawned, interacted with via effects, emits lifecycle notifications, and is
terminated — and that ONE machinery serves userspace effects, capability attenuation, overlays, roles,
middleware, introspection, local processes, websocket clients, and websocket servers alike.

The litmus test the operator gives for the kernel/userspace line: **if a protocol can be modeled over
multiple transports, it belongs in userspace.** JSON-RPC works over ws / stdin / TCP / http, so
JSON-RPC framing — and MCP on top of it — is a userspace handler concern, NOT a host mechanism. The
host surfaces raw transport frames (opaque bytes over a connection, as `Inbound` events); a userspace
handler layers the application protocol. Websockets are stateful + connection-oriented, so the ws
*transport* stays in the host; the router/federator/MCP logic over it is userspace.

A direct consequence the operator drew: **THE OUTPOST collapses into this model.** An outpost is not a
separate host capability — it is a userspace effect handler registered for inbound transport frames
that decides what to do with them (route, federate, spin up local MCP servers). The host needs to know
NOTHING about MCP; it provides only the ws/stdin/process byte-transport + the register-by-string effect
model. This SIMPLIFIES the host (drops any `mcp_client` host executor in favor of a userspace MCP
handler over the generic primitives).

So the document is layered: **Part A** (the foundation, I1–I6) is the handler-session + register +
request/response + reply machinery. **Part B** (I7–I11) are the userspace capability layers built on
it — overlays, roles, middleware, introspection. **Part C** (I12–I14) are the stateful-resource
transport primitives — local process, ws client, ws server — each a handler-session over a host-owned
byte-mover. Parts B and C are strictly ADDITIVE on Part A and do NOT change the I1/I2 kernel shape the
harness owners are already building against.

### The layer model: one Reducer contract, two implementation kinds (operator ruling 2026-08-08)

> Operator: *"the reducer is a host trait, not a wasm session. So the contract can be implemented with
> whatever the host wants."* — resolving *"Do we really need the Executor trait anymore … it might be
> better to just make them the same."*

The capstone forces a clean answer to "where does the host-native mechanism layer sit?" — and the
answer collapses two concepts into one. The **`Reducer` is a host-side TRAIT/contract**, and it has
**two implementation kinds**:

- a **wasm guest session** (a `.wasm` reducer component — the usual case), and
- a **host-native reducer** (Rust code that satisfies the same contract and holds the REAL OS/SDK
  capabilities a wasm reducer cannot — open a socket, spawn a process, call an SDK).

So the old `Executor` trait is NOT a separate layer — a host executor IS a **host-native reducer
implementation**, a terminal handler session that happens to be written in Rust. Both kinds are
dispatched by the SAME machinery (family resolution → forward → `effect/reply` settle → lifecycle →
hash-ids). This is the clean unification: "everything is a session" holds all the way down, and the
host's capability layer is simply the set of reducers whose implementation is native. A leaf effect
(`shell`, `ws`, `proc`) is a host-native reducer registered for its family; a userspace handler
(`weather`, `date`) is a wasm reducer — but the caller, the routing, the authz, and the reply path see
no difference. **Consequence for the build:** the `Executor` trait and the `CompositeExecutor` router
are re-framed as "the registry of host-native reducer impls" and can converge toward the reducer
contract over time (`v-agent-harness-host` is scoping the `Executor` → host-native-reducer collapse);
Part A ships against today's `Executor` trait unchanged, and the collapse is a later refactor the model
now sanctions rather than a prerequisite.

**Why the collapse is contract-safe (v-agent-harness, the `Reducer`-contract owner, verdict
2026-08-08).** The unification needs NO kernel contract change, because the true `Reducer` invariant is
NOT "all state lives in `kv`" — it is a DETERMINISM rule: `fold(&self, event, kv) -> FoldOutput` must be
a pure function of `(event, kv)` (the same input always yields the same kv-mutation + emitted effects).
`kv` is the durable, REPLAYED state; `&self` is immutable per the trait and holds no cross-call state
that fold's OUTPUT depends on. So a host-native reducer's `&self` MAY hold live, non-replayable Rust
capabilities (open sockets, a ws-sink map, SDK clients, blob handles) — provided they never make fold's
output depend on non-replayable live state. This is safe under replay for a concrete reason verified in
the kernel: **replay never re-performs effects.** `replay()` re-runs `fold` but IGNORES the effects it
emits (the results are already on the log), and an effect's outcome is READ from the logged
`EffectResult`, never recomputed from live state (kernel.rs replay + drive-loop). So a host-native
reducer's live capability is TOUCHED ONLY on the live forward path (when the effect first performs, its
result logged right then) and is INVISIBLE to replay (replay needs only the fold logic + the logged
results). The single rule for the host adapter: the wrapped capability's live `&self` state must be
RE-ACQUIRABLE on host restart (re-open the socket, re-create the SDK client) — exactly as an `Executor`
is reconstructed at daemon boot, NOT replayed — and must affect only the logged OUTCOME, never a hidden
fold-determinism input. (`Executor::perform` is `&mut self` while `Reducer::fold` is `&self`; the
adapter bridges this with interior mutability — `RefCell`/`Mutex` over the capability — or keeps the
host-native variant `&mut self` internally, a host impl detail invisible to the kernel-visible
determinism contract.) The proposed gate for the collapse: a host-native reducer holding a live handle,
replayed, yields the same `kv` — pinned once v-ah-host's adapter shape is concrete.

## What it is

A **handler session** is an ordinary session (§3 reducer + log + KV — no new session type) whose job
is to SERVE an effect family. It:

1. **Registers** an effect family in the Global Name Service (`effect/<family>` → its own
   `SessionId`), authz-gated so only a session with authority over that name may claim it (anti-hijack,
   exactly like `store/set` on a `system/…` name).
2. **Receives** effect requests from OTHER sessions as `Inbound` events (the request the caller emitted,
   forwarded by the host with the caller-id + effect-id + a reply-token in the framing), folds them
   through its own reducer against its own state/interface, and MAY perform its own internal effects to
   satisfy them.
3. **Replies** by settling the caller's pending effect — the caller resumes exactly as if a host
   executor had served the family.

The security property falls out **structurally**, not from a new check:

- A **caller** is authorized on the NARROW family it emits (`weather`, `date`) at its target, by the
  kernel's existing SEC-F1 gate — and NEVER on anything the handler does internally. A caller with no
  `weather` grant is denied before the request ever reaches the handler.
- The **handler's** internal effects (`shell(target="date")`) are authorized against the HANDLER's OWN
  capability set, in the HANDLER's OWN drive loop — a completely separate authorization pass. The
  caller's grants are irrelevant to it.
- So the broad grant lives with one audited handler session; callers hold only the narrow family grant;
  and there is NO path for a caller to reach the broad grant except through the narrow registered
  interface. That IS §20a capability attenuation, now with the rescoping unit being a stateful session.

This makes the platform **extensible without kernel edits**: a new effect family = spawn a handler
session + register a name + grant callers the narrow family. The kernel's register-by-string effect
model (routing/authz key on the family STRING, not an enum — `effect_ct`, `EffectRequest::new_with_family`,
`CompositeExecutor::with_effect`) already admits arbitrary family strings; this design supplies the ONE
missing piece — a family whose executor is a guest session rather than host-native code.

## The four pillars → existing seams

| Operator pillar | Realized by |
|---|---|
| (1) userspace-defined effects registered in the name server | register-by-string extension families (`new_with_family`) + a new `effect/<family> → SessionId` name-server registration (I1), authz-gated write authority |
| (2) effect handler = another session/reducer with its own state + interface | a normal `Session`; the request arrives as an `EventBody::Inbound`, folded by the handler's reducer against its KV; no new session kind |
| (3) privileged handler scopes down permissions (the date example) | structural: kernel gates the CALLER on the narrow family (SEC-F1); the handler's broad internal effect (`shell`) is gated against the HANDLER's grants in its own loop — §20a resource-rescoping, stateful form |
| (4) handler registers its own resources to do authz on | the family's `target` = the handler's virtual sub-resource (`date.now`); coarse Cedar gate on `(family, target)` + optional fine-grained in-fold authz; handler registers Cedar entities for its family (I5, with `design-dogwood-cedar`) |

## The request/response protocol (the crux)

A caller emitting a userspace-family effect must end up SUSPENDED on that effect (keyed by its
`EffectId`) and RESUME when the handler replies — identical in shape to any routed effect, so the
reducer author writes nothing special. The wiring:

```
 caller session                    kernel drive loop                 host                         handler session
 --------------                    -----------------                 ----                         ---------------
 emit Effect{                  →   1. is_control? no                                              
   family="weather",               2. authorize(caller,                                           
   target="today",                    action="weather",                                           
   payload=...}                       resource="today")   ← SEC-F1 NARROW gate (deny → caller done)
                                   3. route: family resolves                                       
                                      in effect registry →                                         
                                      Dispatched frame,      →  UserspaceEffectExecutor.perform:   
                                      effect stays OPEN          resolve effect/weather → handler
                                      (I2 Deferred)              SessionId; forward as Inbound  →   fold Inbound{
                                                                 {effect-request, payload,             effect-request,
                                                                  caller-id, effect-id,                caller-id, ...}
                                                                  reply-token}                         → do own effects
                                                                                                        (shell, http…
                                                                                                         gated on
                                                                                                         HANDLER grants)
                                                                                                      → emit Effect{
 resume: fold        ←  4. settle_effect_result(caller,   ←  ReplyExecutor: validate            ←        family=
   EffectResult{           effect-id, Ok(response))           reply-token, then                          "effect/reply",
   id=effect-id,           folds EffectResult back            settle by (caller, effect-id)               target=
   payload=response}       to caller's continuation                                                       reply-token,
                                                                                                          payload=resp}
```

Three properties make this sound:

- **Correlation by `EffectId`** (S4, already the kernel's rule): the caller resumes only on the
  `EffectResult` carrying its effect-id. The reply-token the host minted on forward is the CAPABILITY
  to settle exactly that `(caller, effect-id)` — the handler cannot forge a reply to any other
  session/effect (confused-deputy / reply-forgery defense; the token is the §12c per-effect bless,
  reused as a reply-authority).
- **Deferred settlement** (I2, the one new kernel seam): the executor's `perform` can return
  `Deferred` instead of an immediate `EffectOutcome`, leaving the caller's effect OPEN; the host later
  settles it by id. This GENERALIZES the existing `control/signature` fold-back path
  (`is_fold_back_control` + `settle_control_result`), which already does exactly "give it a Dispatched
  frame, host answers off-band, settle back by id" — we lift that from a hardcoded control-family
  special-case to a general primitive available to any routed family.
- **Handler statefulness is free**: the handler is a session, so its KV persists across requests, its
  log is durable/auditable, and it can itself emit userspace effects (a handler calling a handler —
  the cause-DAG records the chain). It IS "just another session/reducer."

## Part A — the foundation: handler-session + register + request/response + reply (I1–I6)

Each increment is independently green and gated. I1–I2 are `cdz-kernel` (v-agent-harness); I3–I4 are
`cdz-agent-host` (v-agent-harness-host); I5 is authz (deferred to the operator — see below); I6 is
lifecycle. **This part is FROZEN and being built** — I1/I2 shape is ratified with v-agent-harness
(executor-returned `EffectOutcome::Deferred` + generalized `settle_effect_result`) and v-ah-host has
reviewed I3/I4/I6 favorably. Parts B and C build strictly on top of it.

> **I5 authz — DEFERRED TO THE OPERATOR'S RETURN (ruling 2026-08-08).** The operator will not spin up
> `design-dogwood-cedar` again until back — the temporal-Cedar resource model is "too complicated for
> async, likely needs upstream Cedar PRs I want to open myself, not an agent." So I1–I4 proceed against
> the AUTHZ DEFAULT (kernel registration gated via the existing `NameStore` prefix authority = the
> interim security posture), and I5 (the full handler-registered Cedar resource model) slots in on the
> operator's return. **Hard constraint on I1–I4: do NOT foreclose the eventual Cedar resource model —
> keep the authz seam OPEN/PLUGGABLE** (the `effect/reply` + delegating-executor authz must call a
> pluggable authorizer, never bake policy) so I5 drops in without rework.

- **I1 — kernel: the `effect/*` registration namespace + resolution.** Add an `effect/` name-server
  namespace: `effect/<family>` is a pointer name whose value is a handler's `SessionId` (= genesis
  hash), written via the existing `store/set` and gated by a new `effect/` write-authority prefix in
  `NameStore::authority_prefix_of` (`name_store.rs`) — only a session Cedar-granted `store/set` over
  `effect/<family>` may claim/repoint it (the anti-hijack surface, mirrors `system/`). Add a resolver
  `NameStore::resolve_effect_handler(family) -> Option<SessionId>` and a family predicate
  `is_registered_effect_family` (a family that is not a well-known partition and resolves in the
  registry). *Gate:* a kernel unit test — register `effect/weather → H`, resolve it back; an
  unauthorized `store/set effect/weather` denies. **Anchors:** `name_store.rs` (`NameAuthority`,
  `authority_prefix_of`, new resolver), `effect.rs::effect_ct` (an `EFFECT_REGISTRY_PREFIX = "effect/"`
  const + `is_registered_effect_family`).

- **I2 — kernel: deferred effect settlement (the one new primitive).** Generalize the
  `control/signature` fold-back into a first-class deferred path: an `EffectOutcome::Deferred` (or a
  `perform` return that signals "open, settle later"), plus `Session::settle_effect_result(effect_id,
  outcome)` promoted from the control-only `settle_control_result` so ANY open routed effect can be
  settled asynchronously by id, replay-safe (the settlement is a durable `EffectResult` event carrying
  the id, exactly as today). The host mints a per-forward **reply-token** bound to `(caller SessionId,
  EffectId)`; the kernel carries it as opaque bytes on the forwarded request (never interprets it — §12c
  per-effect bless). *Gate:* a kernel unit test — dispatch an effect that returns `Deferred`, confirm
  the effect is `open` (not settled), then `settle_effect_result` and confirm the caller folds the
  `EffectResult` and resumes; a replay of the log reproduces the same state. **Anchors:** `executor.rs`
  (`EffectOutcome`), `kernel.rs` (`drive_worklist` deferred arm — reuse the fold-back branch;
  `settle_control_result` → generalized `settle_effect_result`), `effect.rs` (`is_fold_back_control`
  subsumed by the general deferred path).

- **I3 — host: the delegating executor.** A host `UserspaceEffectExecutor` registered in the
  `CompositeExecutor` whose `handles_family(f)` is true when `resolve_effect_handler(f)` is `Some`
  (mechanism = "a handler is registered"). Its `perform` (a) resolves the handler `SessionId`, (b)
  forwards the request into that session's `Inbox` as an `EventBody::Inbound{content_type:
  "effect-request/<family>", payload}` with the caller-id + effect-id + reply-token in the framing
  (reuses the `EmitExecutor` inbox-delivery machinery + the on-loop `&mut AgentHost` executor pattern),
  and (c) returns `Deferred`. A family with no registered handler → immediate `EffectOutcome::Err`
  ("no handler for <family>", fail-loud, like an unregistered family today). *Gate:* a host test — a
  caller emits `weather`, the framing lands in the handler's inbox with the correct correlation fields;
  an unregistered family Errs. **Anchors:** `cdz-agent-host/src/` (new `userspace_effect_exec.rs` beside
  `emit.rs`/`fs_exec.rs`), `factory.rs` (register it), `async_host.rs` (inbox delivery).

- **I4 — host + vocab: the reply path.** An `effect/reply` family the handler emits to answer a request:
  `target` = the opaque reply-token it was handed; `payload` = the response bytes. A host `ReplyExecutor`
  validates the token (it must name a `(caller, effect-id)` the host actually forwarded and has not yet
  settled — one-shot), then calls `settle_effect_result(caller, effect-id, Ok(response))`, folding the
  `EffectResult` back to the caller. Possession of a valid reply-token IS the authority (like a `ws`
  conn-id or an `Emit` peer-id — the target is an opaque host-minted handle the guest echoes back), so
  `effect/reply` is token-authorized rather than needing a broad grant. A malformed / stale / foreign
  token → the reply is refused (the handler cannot settle effects not routed to it). *Gate:* a host E2E
  — full round-trip: caller emits `weather` → handler folds → handler emits `effect/reply` → caller
  resumes with the response; a forged token is refused; a double-reply is refused (one-shot). **Anchors:**
  `effect.rs::effect_ct` (`EFFECT_REPLY = "effect/reply"` + safe-logging), `cdz-agent-host/src/`
  (`ReplyExecutor`).

- **I5 — authz: handler-registered virtual resources (with design-dogwood-cedar).** Two-layer authz,
  wired so the handler "registers its own resources to do authz on":
  - COARSE (mandatory, kernel/Cedar, deny-by-default): the caller's emit of `(family=weather,
    target=today)` is gated by the swappable Cedar policy — `action == "weather"`, `resource ==
    "today"`. The handler's virtual sub-resources ARE the effect targets, so a policy can scope which
    callers reach which sub-op. The handler REGISTERS these as Cedar entities (content-addressed policy
    artifacts named on the log, §20b) so operators can author `permit(action=="weather") when resource
    in Weather::PublicForecasts`.
  - FINE (optional, in-fold): the handler sees the caller-id + target in the forwarded framing and MAY
    apply application-level authz its own way (rate limits, per-caller quotas, business rules) before
    performing its internal effect. This is ordinary reducer logic — no kernel surface.
  Coordinate the entity/resource schema with `design-dogwood-cedar` (temporal-Cedar authz). *Gate:* a
  Cedar E2E — a caller with the narrow grant reaches the handler; a caller without it is denied at the
  kernel; a policy scoping `resource` admits `date.now` but denies `date.set`. **Anchors:** the Cedar
  `ComponentAuthorizer` (`wasm_host.rs` action-map already = `content_type.family`), dogwood-cedar
  resource model.

- **I6 — lifecycle: handler-session lifecycle + registration GC.** A handler is spawned via
  `lifecycle/spawn` (it registers its `effect/<family>` at genesis or on first tick) and torn down via
  `lifecycle/terminate`. On terminate, its `effect/<family>` registration is pruned — reuses the
  session-directory death seam (the terminal `Terminated` event drives a `store/remove` / repoint of
  the `effect/` name). While a handler is down or mid-restart, an inbound request to its family folds
  back `Err(handler-gone)` — no different from `ws/send` to a closed conn or an `Emit` to a terminated
  session (the existing bounce path). *Gate:* a host test — register a handler, terminate it, confirm
  the registration is pruned and a subsequent caller effect folds `Err`. **Anchors:** `lifecycle/*`
  (session-lifecycle design, already landed), the directory death seam.

## Part B — userspace capability layers: overlays, roles, middleware, introspection (I7–I11)

These are the operator's 2026-08-08 extensions. Each is "just another reducer/session" over the Part A
machinery — none changes the I1/I2 kernel shape. They make effect resolution SESSION-SCOPED and
COMPOSABLE, which is the mechanism that makes capability attenuation ergonomic (you install a scoped
handler set FOR a session, rather than only globally).

### I7 — effect OVERLAYS (per-session effect resolution)

> Operator: *"attach effect overlays to sessions … a session could have a different set of handlers …
> from the global set."*

An **overlay** is a per-session mapping `family → resolution` that SHADOWS the global registry for that
session. Effect resolution for a session becomes an ordered lookup: **session overlay → default system
overlay (I8) → global `effect/<family>` registry**. First match wins. So a session can be given a
DIFFERENT handler for `weather` (e.g. a mock in a test session, or an attenuated handler for an
untrusted session) without touching the global registration.

- **Value model:** an overlay is itself content-addressed and named on the log — `overlay/<name> →
  Hash` where the artifact is a set of `(family → handler-SessionId | Disabled)` entries. A session's
  active overlay is a pointer on the session's own namespace (`session/<id>/overlay → Hash`), set at
  genesis (spawn provides it) or via an authorized `store/set`.
- **Resolution** is a kernel/host lookup change ONLY at the point I3's `UserspaceEffectExecutor`
  resolves a family to a handler: instead of `resolve_effect_handler(family)` reading only the global
  `effect/<family>`, it walks the session's overlay chain first. Pure, replay-safe (the overlay pointer
  is frozen into the session's log like any resolved name).
- **Disable (operator addition 2):** an overlay entry may be a NEGATIVE marker `family → Disabled` — the
  effect is REMOVED for this session, so performing it folds `Err(effect-unavailable)` rather than
  falling through to global. This is how a session is denied a capability by shadowing, not just by
  authz.
- **Composition:** multiple overlays compose as an ordered stack (session-attached overlays in
  attach-order, then default, then global); first non-`Disabled` match wins, a `Disabled` short-circuits.
  *Default ⟨D8⟩:* a session has ONE active overlay artifact (which may itself be composed at
  registration time from several by a builder), keeping runtime resolution a single chain walk; a
  multi-overlay runtime stack is additive later.

*Gate:* a host/kernel test — a session with an overlay resolves `weather` to the overlay handler, not
the global one; a `Disabled` entry folds `Err(unavailable)`; no overlay falls through to global.
**Anchors:** `name_store.rs` (`overlay/`, `session/<id>/overlay` names), the I3 resolver (overlay-chain
walk), `effect.rs::effect_ct` (`OVERLAY_PREFIX`).

### I8 — DEFAULT system overlay (born-with)

> Operator: *"install a default overlay in the system as well that each session gets on spawn by
> default, even if it does not configure an overlay."*

A system-wide `overlay/system-default → Hash` that EVERY session's resolution consults after its own
overlay and before the global registry. Installed/configured system-wide (a `system/`-authority
name, set at genesis or by an authorized operator write, §20b swappable-by-hash). This is where the
platform ships a baseline handler set (the "date" rescoping handler, the standard tool handlers) so a
plain session gets sensible capabilities without configuring anything. *Gate:* a session with NO own
overlay still resolves families provided by the default overlay. **Anchors:** genesis bootstrap (§3),
`overlay/system-default` name.

### I9 — ROLES (named handler-group bundles)

> Operator: *"install a group of effect handlers with a single name … those could be roles … a
> developer role is going to need different effects from a PM role."*

A **role** is a NAMED, reusable overlay bundle: `role/<name> → Hash` where the artifact is a set of
`(family → handler)` entries. Attaching a role to a session = attaching its bundle as an overlay. So a
role is a first-class, shareable capability set (developer vs PM vs reviewer). Roles compose (a session
may attach multiple; ⟨D9⟩ *default:* attach-order precedence, a later role shadows an earlier one for a
shared family; explicit per-family overlay beats any role). Registration + composition live in the name
server exactly like overlays — a role IS an overlay with a well-known `role/` naming convention +
intended reuse across sessions. *Gate:* attach `role/developer`, confirm its families resolve; two
roles with an overlapping family resolve by precedence. **Anchors:** `role/` names, the I7 overlay
resolver (a role is an overlay source).

### I10 — effect MIDDLEWARE (transparent request/response interposition)

> Operator: *"effect middleware, which is really just another reducer that is able to transparently
> intercept effect requests and responses and either transform them or log them or whatever it wants."*

**Middleware** is a handler session interposed on the effect path that sees the REQUEST before it
reaches the terminal handler and the RESPONSE before it returns to the caller, and may transform, log,
or pass through unchanged. It is the SAME "handler is just a reducer" primitive, but CHAINED rather than
terminal: a middleware handler forwards to the next link (via the same `UserspaceEffectExecutor` forward
+ `effect/reply` settle), so the caller → [middleware…] → terminal-handler → [middleware…] → caller
chain is built entirely from Part A's forward/reply machinery — no new kernel seam.

- **Registration + ordering:** middleware is an overlay-chain concept — a family's resolution can be a
  STACK of middleware links terminating in the real handler (`family → [mw1, mw2, handler]`). Registered
  in the same overlay/role artifacts (a middleware entry is a handler that is declared as forwarding).
- **Transparency guarantee:** a middleware that does nothing forwards the request unchanged and relays
  the reply unchanged — structurally, it re-emits the same effect to the next link and pipes back the
  `effect/reply` it receives. The correlation is per-link `EffectId`s chained by the reply-token, so a
  middleware cannot see or settle effects outside its link.
- **Attenuation synergy:** middleware is the natural place for a userspace authz/log/rate-limit layer
  (it can enforce or record policy on every request/response) — complementary to the mandatory kernel
  gate, not a replacement.
  ⟨D10⟩ *default:* middleware is a per-family ordered stack declared in the overlay artifact; a
  global/all-families middleware is additive later.

*Gate:* a pass-through middleware is transparent (caller gets the terminal handler's exact reply); a
transforming middleware alters request/response as declared; a middleware cannot settle a foreign
`EffectId`. **Anchors:** the I7 overlay artifact (middleware-stack entries), I3/I4 forward+reply (reused
per link).

### I11 — effect INTROSPECTION (contracts + docs + type signatures, in binary AST)

> Operator: *"reducers need to be able to get the full list of effect contracts with their
> documentation and type signatures. All of this should be in the binary AST format."*

A reflection capability: a reducer queries the FULL LIST of effect contracts a session can reach (its
resolved overlay chain + global registry), each entry carrying the family, its DOCUMENTATION, and its
TYPE SIGNATURES — all encoded in the **binary AST format** (the one `cdzast` doc model, HARD operator
constraint). This extends the existing `control/capabilities` manifest (which today reports
family + grant-state + scope) and the `control/signature` component-reflection path (which already
reflects export types) into a full contract catalog.

- **Mechanism:** a new `control/effects` (or extend `control/capabilities`) control-plane query
  (authz-exempt, kernel/host-answered, fold-back like `control/signature`). The answer is a
  content-addressed binary-AST document listing each reachable effect's contract. A handler PUBLISHES
  its own contract (doc + type sig) as a binary-AST artifact named under its `effect/<family>` (this is
  D7 from Part A, now first-class); introspection aggregates the reachable set.
- **Binary AST:** coordinate the contract/doc/signature encoding with `design-cadenza-docs` (binary-AST
  doc model) + the binary-ast-dict work + v-agent-harness's signature-query descriptor — REUSE those
  encoders, do not invent a parallel format.

⟨D11⟩ *default:* introspection is a control-plane query returning a binary-AST catalog of the session's
RESOLVED reachable effects (post-overlay), so a reducer sees exactly what it can actually call. *Gate:*
a reducer queries the catalog, receives a binary-AST doc whose entries match its resolved overlay chain,
each with doc + signature; the round-trip decodes. **Anchors:** `control/` query family, `event_ast`
(binary-AST encode, reuse signature-query descriptor), design-cadenza-docs doc model.

### I11b — SCHEMA-HASH effect identity (content-addressed contract versioning)

> Operator: *"I wonder if effects should be identified by hashes of their schemas. That makes it easier
> to expand effect contracts over time since it does not invalidate any old messages."*

> **⚡ seq367 — schema-identity is UNIVERSAL; it SUPERSEDES family-string routing everywhere in this doc
> (including built-ins).** Operator seq367 (2026-08-10, via v-agent-harness): effects are identified by
> SCHEMA, not string NAME, and the 6 built-ins (`shell`/`http`/`model`/`now`/`timer`/`emit`) migrate to
> schema-identity too. So wherever earlier sections (What it is, the four-pillars table, I1) describe
> routing/authz keying on the family STRING (`effect_ct` consts, `new_with_family`,
> `effect/<family> → SessionId`), read the KEY as the **schema-hash**: the family string becomes a HUMAN
> ALIAS declared IN the schema (D14=A) for logging/manifest/policy-readability, resolved FROM the schema,
> never the routing/authz key. This removes the built-in carve-out those sections carried (built-in
> `effect_ct` strings were treated as durable wire keys); post-seq367 there is no carve-out — the wire is
> a bare schema-hash for ALL families (D14=A), registration is `effect/<schema-hash> → SessionId`, and
> `handles_family` becomes `handles_schema`. The `effect_ct` string consts survive only as the
> alias/display table (schema-hash → name), not as identity. This is the clean end-state of D13/D14=A/D15;
> the build stages it incrementally (extension families first, then the built-in migration — see the
> migration note in D15/D14). Everything below in this section is that model; the seq367 note just states
> it applies UNIVERSALLY, not only to extension families.

This holds up and fits the platform's everything-hash-identified through-line (sessions = genesis hash,
blobs = content hash, ws-conns = a hash). An effect's SCHEMA — the operation signatures + type contract
that I11 already makes an introspectable, binary-AST artifact — gets a content **hash**, and that hash
is the effect CONTRACT VERSION's stable identity. The benefit the operator cites falls out: because a
message references the schema-hash it was built against, EXPANDING a contract (adding an op, a field)
mints a NEW schema-hash while old messages keep resolving to the old schema — content-addressed,
naturally immutable, self-versioning. No message is ever invalidated by a contract growing.

**The schema IS a binary-encoded cdzast AST (operator, everyone agrees).** An effect's schema is
represented AS a binary-encoded `cdzast` AST artifact — the SAME one `cdzast` binary-AST model
`design-cadenza-docs` owns and I11 introspection reports — and the **schema-hash is the content hash of
those canonical binary-AST bytes.** Confirmed with `v-syntax` (owns `cadenza-ast`, the cdzast codec),
no gap:
- A schema encodes via `cadenza_ast::codec::encode` as a name-headed tree, e.g.
  `(effect Weather (op today (-> Unit Forecast)) (op set (-> Forecast Unit)))`; the codec round-trips
  ANY such tree byte-identically (structure-only, TOTAL decode, `cdzast\x00\x01` header). The
  per-op type SIGNATURES within the tree are produced by `ast_marshal::build_type` (the landed wit-Type
  → structural type-descriptor AST used by signature-query / `component_signature`) — so the effect
  schema is the RICHER cdzast tree (declared name per D14=A + op signatures + the D15 authz contract)
  and `build_type` COMPOSES INTO it to encode each op's type, rather than being the whole schema. Hash
  the whole cdzast effect-tree = the identity (v-agent-harness seq367; reconcile the `build_type` ↔
  cdzast-tree split with v-syntax + the `ast_marshal` owner).
- **schema-hash = content hash of the canonical `encode()` bytes** — hash the canonical `\x00\x01`
  bytes (call `encode()` then hash), NOT a re-derived form. Per the `Event::hash` precedent, treat the
  schema-hash as an identity STABLE across cdzast container-format evolution (the `\x00\x01` canonical
  bytes are format-pinned, so equal schemas always hash equal and an identity does not shatter on a
  format bump). The primitive shape is settled with v-syntax (concierge floor (B) keep-cadenza-ast
  algo-free): the content-hash input IS `cadenza-ast`'s existing `codec::encode(arenas) -> Vec<u8>` (the
  canonical cdzast `\x00\x01` bytes — the one thing that could drift, already single-sourced there), and
  the harness/kernel computes the effect-schema content hash as `Hash::of(encode(&schema_ast))` —
  `Hash::of` being the ONE unified content-address (blake3, operator 2026-08-08), so there is no
  per-caller re-implementation and cadenza-ast stays dependency-light (no blake3 in the bottom crate). No
  wrapper alias is introduced: `encode` is directly the canonical-content-bytes primitive (its doc
  carries the content-addressing role; operator 2026-08-09 — a same-body rename wrapper was rejected).
- **The meta-schema is evolvable (operator).** The schema-for-the-schema — the AST node vocabulary used
  to REPRESENT a schema — must itself grow without a hard break. It does, by construction: a new
  contract-description construct is a new HEAD NAME + list shape = NO codec change (tolerant-reader — an
  older reader sees an uninterpreted list, never breaks). Only a genuinely new LEAF value kind (beyond
  Int/Float/Str/Char/Bytes/Bool/Name/Sym/Decimal) needs a container bump, a drop-in `\x00\x02` the
  header + `BadTag` totality already support. So the node vocab is additively extensible — the same
  tolerant/additive discipline the effect contracts get from schema-hashing.

How it composes with what's already designed:

- **`ContentType` — schema-hash REPLACES `version: u32` (operator ruling on D13; clean break, NO
  backward compat).** Today `ContentType` is `{family, version: u32}` with a tolerant-reader model
  (`matches_family` ignores version; `version_in` range-checks). The operator ruled (verbatim: *"I do
  not want to preserve any kind of backward compatibility. Rip out the old versioning crap."*): the
  `version: u32` axis is REMOVED entirely and schema-hash becomes the contract-version identity for ALL
  families — built-in and extension alike. (D13's interim shape was `ContentType {family, schema_hash}`;
  **D14=A below then eliminated the struct entirely** — the wire is a bare schema-hash and `family` is a
  SCHEMA field. Read this bullet as the version-axis removal; D14=A is the final envelope shape.) This is
  a BREAKING `ContentType` change, which is fine + preferred: there is no deployed durable state (v0), so
  a compat shim is exactly the "old thing that never gets cleaned up" the no-migration-cruft directive
  rejects. The tolerant-reader rule re-expresses cleanly: match `family`, then check whether you
  understand the `schema_hash` (you hold that schema) instead of a numeric `version_in` range — a known
  family at an unknown schema-hash is the "known family, unknown contract" case the reader defers/rejects
  honestly, never misdecodes. `v-agent-harness` owns `ContentType`; coordinate the removal with them.
- **Registration (I1):** `effect/<family>` resolves to the handler `SessionId`, AND the handler
  publishes its current schema (a binary cdzast AST artifact) whose hash is the contract-version id. The
  family name resolves to the CURRENT schema-hash; a caller can address "the `weather` handler" (latest)
  or pin "`weather` at schema-hash H" (a specific contract), and old schema-hashes stay valid (the
  handler keeps serving prior contract versions it still understands, else fails a pinned request
  `Err(unsupported-schema)`). The family stays the register-by-string routing/human/Cedar-action key;
  schema-hash is the exact-contract key ALONGSIDE it (this is orthogonal to removing `version` — the
  family string is untouched; it is the `version` NUMBER that schema-hash replaces).
- **Introspection (I11):** the schema IS the introspectable contract, so hashing it is natural — the
  I11 catalog reports, per effect, its `family` + its current `schema_hash` + the binary cdzast AST
  schema itself. Version negotiation: a caller reads the handler's advertised schema-hash(es) via
  introspection, picks one it understands, and stamps its requests' `ContentType.schema_hash` with it;
  the handler dispatches on the stamped schema-hash.

⟨D13 — RESOLVED by the operator; then SUPERSEDED by D14=A⟩ schema-hash **REPLACES** `version: u32` (no
third field, no `version` kept, no backward compat — rip the u32 version axis out entirely). D13's
interim framing had `ContentType` become `{family, schema_hash}`; **D14=A (below) then went further and
ELIMINATED the `ContentType` struct entirely** — the wire content-identity is a BARE schema-hash and the
`family` is a SCHEMA field, not a wire field. So the final shape is: wire = one `Hash`; the schema (a
binary cdzast AST; schema-hash = the content hash of its canonical `\x00\x01` bytes) is the single
self-describing source of name + contract + authz. *Gate (when built):* the `version: u32` field is gone from `ContentType`; a message stamped with
schema-hash H resolves + folds against schema H; expanding a contract to H' leaves H-stamped messages
resolving to H; a message at an unknown schema-hash fails honestly (`Err(unsupported-schema)`), never
misdecodes; equal schemas hash equal across a cdzast container-format bump. **Anchors:** `event.rs`
(`ContentType` — remove `version`, add `schema_hash: Hash`; update `matches_family`/drop `version_in`),
`cadenza-ast` (`codec::encode` + `Hash::of` — no wrapper alias, operator 2026-08-09), `hash.rs`, I1 registration
(name → current schema-hash), design-cadenza-docs (the shared cdzast schema representation).

### D14 — does `ContentType` collapse to JUST the schema-hash? (operator, going past D13)

> Operator: *"Wouldn't the content type just be the hash of the schema? I don't even know why we would
> continue to have the content type at all when the schema is self-describing of the content inside."*

The operator is right that the ENVELOPE collapses — but the analysis shows a **stable name is still
load-bearing, and it relocates INTO the self-describing schema rather than disappearing.** Working
through everything the `family` string did that a bare schema-hash cannot:

- **Routing (which handler serves this) — the decisive case.** A schema-hash is by construction NOT
  stable across contract evolution (that is its whole point: expand the contract → new hash). But
  routing needs a key STABLE across versions: an OLD message carries `H_old`, the handler has since
  evolved to `H_new`, and "deliver this to the weather handler" must still resolve. Routing by hash
  ALONE cannot do this — `H_old` names a past contract, not "the current weather handler." So either
  the registry maps every historical schema-hash → the current handler (unbounded, and still needs a
  notion of "same effect across versions" = a stable identity by another name), OR the stable name
  lives somewhere. **This is the genuine reason a name survives** — exactly the flag the operator asked
  for.
- **Cedar authz action key — also decisive.** Policies are authored by humans against STABLE action
  names (`permit(action=="weather")`). If the action key were the schema-hash, every contract expansion
  would silently invalidate every policy (the hash moved). So the authz action must be a stable name,
  not a hash.
- **Human readability** — derivable from introspecting the schema; NOT load-bearing on its own.

So both decisive jobs need a name that survives schema evolution. The operator's insight still holds,
though: because the schema is SELF-DESCRIBING, that name does not need to be a SEPARATE envelope field
— **the schema declares its own family/name as a field** (`(effect Weather (op today …) …)` — the
`Weather` head IS the stable name, right there in the schema). Then:

- The **content identity on the wire is JUST the schema-hash** (a single `Hash`) — the operator's
  collapse is achieved; there is no separate `{family, schema_hash}` struct riding every message.
- The stable **family/name is RECOVERED by introspecting the schema** the hash addresses (resolve
  `schema_hash → schema AST → its declared name`). Routing and Cedar key on that declared name; it is
  stable across versions because successive schema versions of the same effect declare the SAME name
  (only their ops/types grow, minting a new hash while the declared name is unchanged).
- Registration inverts accordingly: `effect/<name>` (the stable declared name) → the current
  `schema_hash` → the handler. An old `H_old` message routes by resolving `H_old`'s schema, reading its
  declared name, and looking up the current handler for that name.

⟨D14 — DECIDED = A (FULL COLLAPSE), operator⟩ The operator chose the full collapse (verbatim: *"put
the whole thing in the schema. We have the whole s3fifo cache so the perf is not a problem."*). LOCKED:

- **The wire content-identity is a BARE schema-hash — a single `Hash`. The `ContentType` STRUCT is
  ELIMINATED.** There is no `family` field and no `version` field on the wire; the content-identity an
  effect/event carries is exactly one schema-hash.
- **The self-describing schema is the single source** for everything the type used to carry: the stable
  name (the `(effect Weather …)` head), the type contract (op signatures), and the authz contract
  (D15). Routing resolves `schema-hash → schema → declared name`; Cedar keys on the hash-identified
  schema (D15).
- **The perf concern that motivated the thin-`{family, schema_hash}` alternative (B) is a NON-ISSUE:**
  the S3-FIFO cache makes `schema-hash → schema` resolution cheap, so routing does the lookup through
  the cache with no meaningful cost. There is no per-message schema-lookup penalty worth a wire field.

This composes with everything already locked: D13 (schema-hash replaces `version: u32`), the
self-describing schema (name-in-schema), and D15 (authz contract in schema / grants external /
hash-keyed / no-auto-carry / opt-in `supersedes` lineage). **The ENTIRE schema-identity + authz arc is
now settled:** the wire is just a schema-hash; the schema is the single self-describing source of
name + type-contract + authz; resolution goes through the s3fifo cache. This is a breaking envelope
change owned by `v-agent-harness` (it folds with their binary-target rework) — the `ContentType` struct
is removed, not merely re-fielded. See the reconciliation note under D13 above (D14=A supersedes D13's
interim `{family, schema_hash}` framing — the family is NOT a wire field, it is a schema field).

### D15 — the schema is self-describing for AUTHZ too; authz is HASH-BOUND (operator thread; anti-collision by construction)

The operator extended D14 into authz across three messages, ending at the maximally-secure model. The
important part is a soundness distinction that must NOT be hand-waved (operator: *"do not hand-wave
it"*). Working the thread through:

1. **Schema self-describes authz facts (operator).** The schema should declare not just its routing
   name but its authz-relevant structure — the stable Cedar ACTION name and the RESOURCE shape (target
   kind, capability class, the predicates a policy matches). So the Cedar action + resource model is
   DERIVED FROM the schema, not a separate registration. This reinforces D14-collapse-A: the schema is
   the single self-describing source for (1) the content/type contract, (2) the routing name, (3) the
   authz action + resource declarations.

2. **The collision soundness question (operator).** If authz keys on a name the SCHEMA declares, two
   DIFFERENT schemas (different hash) could both declare `weather` — and a grant meant for schema-A's
   `weather` would leak to a malicious schema-B that merely declares `weather` too. A bare
   self-declared name is spoofable; something must bind the name to a legitimate identity.

3. **The resolution — bind authz to the HASH-identified schema (operator: "the most secure way").**
   Because the schema-hash covers the schema's authz declarations, a same-name-different-hash collision
   CANNOT cross grants BY CONSTRUCTION: schema-B is a different hash, so a grant bound to schema-A's
   hash-identified authz simply does not apply to it. The name becomes a human/routing label; the
   AUTHORITY is the hash-bound authz. This is stronger than "the name-server registration binds the
   name" — it moves the security root INTO the content-addressed schema, so no registration-authority is
   even needed to PREVENT the collision.

**The one distinction that keeps this sound (must be explicit).** "Authz bound to the schema" must mean
the authz **CONTRACT/shape** is hash-bound, NOT that the handler ships its own **permit-rules**. These
are different:
- **Hash-bind the authz CONTRACT** — the action name, the resource shape, the predicate-relevant
  structure Cedar matches against. This is the handler-author's legitimate self-description, and
  hash-binding it is exactly what kills the collision (a grant references the hash-identified action +
  resource shape; a different schema is a different hash is a different authz identity). ✅ Adopt this.
- **Do NOT let the schema carry the permit-DECISION (who may invoke).** *Who is permitted* is the
  OPERATOR's/caller's policy, not the handler-author's — §20b: policy is the operator's, swappable, on
  the log; the handler provides mechanism, not policy. If a handler shipped `permit(everyone)` inside
  its schema, it would authorize ITSELF — a confused-deputy / self-authorization hole. So the Cedar
  POLICY (the permit/forbid rules) stays the operator's separate, log-named, swappable artifact (I5 /
  §20b); it simply KEYS ON the hash-bound authz identity the schema declares.

So the sound model is: **the schema is self-describing for the authz CONTRACT (hash-bound, unforgeable,
collision-proof by construction); the authz POLICY remains the operator's, keyed on that hash-bound
identity.** The name is a routing/human label with no security weight; the schema-hash + its declared
authz contract is the authority the operator's policy grants against. This resolves the collision
without a registration-authority (the hash-binding does it), while keeping the grant AUTHORITY with the
operator (not the handler-author) — the two together are what make it both collision-proof AND not
self-authorizing.

⟨D15 — SETTLED by the operator⟩ The operator confirmed this model exactly and settled the succession
question. The locked authz model:

- **Grants live OUTSIDE the schema** (operator: *"I do not think the grants should be inside the schema.
  It should definitely live outside of it."*) — confirms the contract-vs-decision split above: the
  permit-decision/policy is a SEPARATE, operator-owned, swappable artifact (§20b), never in the schema.
  No self-authorization/confused-deputy.
- **Authz resources are identified by SCHEMA-HASH** (operator: *"identify authz resources by the schema
  hash and then it is completely unambiguous which resources are being granted."*) — a grant names the
  schema-hash as the resource, so which resource is granted is unambiguous and collision-proof by
  construction. The hash-bound authz CONTRACT is the resource identity; the external policy grants
  against it.
- **Version succession = NO auto-carry; migration-at-registration by the registrant** (operator: *"if a
  new schema gets registered then whoever is registering it can do the migration and include grant
  upgrades."*). A grant to `weather@H1` does NOT auto-apply to `weather@H2`. Registration of a new
  version is the point where grant migration happens — explicit, registrant-driven, never a silent
  widen. So a contract expansion never silently widens an existing grant; the registrant deliberately
  carries grants forward.
- **OPT-IN lineage: bake the predecessor hash INTO the new schema** (operator: *"bake the old hash of
  the previous version into the schema of the new version … to grandfather in old grants."*). A new
  version H2 DECLARES its predecessor H1 as a schema field — e.g. `(effect Weather (supersedes H1)
  (op …) …)`. Because the link is baked in, it is itself HASH-BOUND (part of what H2's hash covers) =
  unforgeable + verifiable. USES: (1) the external authz policy MAY, if it chooses, follow the declared
  H1→H2 lineage to grandfather an H1 grant forward to H2 — opt-in, driven by the successor's own
  declaration + the registrant's migration, NOT a silent auto-carry; (2) a verifiable version chain for
  audit/discovery. Crucially this preserves the security properties: declaring `supersedes(H1)` does NOT
  self-grant — the permit-decision stays external + operator-controlled, so a rogue schema that declares
  `supersedes(H1)` grandfathers in NOTHING unless the operator's policy chooses to honor that lineage.
  It merely OFFERS a verifiable lineage the external policy may consult. It composes cleanly with
  grants-external + hash-keyed-resource (lineage is a schema-declared hint, not an authz grant itself).
- **What registration does now.** With the authz identity hash-bound, name-server registration
  (`effect/<name> → current schema-hash`) is ROUTING/DISCOVERY + namespace-claim only — no longer the
  security root (the hash-binding is). It answers "which handler + which current contract," and may gate
  WHO can claim a routing name (a namespace-ownership concern), but not "who may call" (that is the
  external policy keyed on the schema-hash).
- **Resource-shape vocabulary (I5 detail).** What the schema declares for the Cedar resource model
  (target kind, capability class) must align with the `ResourcePredicate` / Cedar entity model —
  coordinate with the I5 owner + `design-dogwood-cedar` when that work resumes.

**Locked authz model (for the deferred I5 build):** authz CONTRACT hash-bound in the self-describing
schema (collision-proof by construction); **grants EXTERNAL** — the operator's swappable Cedar policy
keyed on the schema-hash resource; **no auto-succession** — the registrant does grant-migration
explicitly at registration; **opt-in lineage** via a hash-bound `(supersedes H_prev)` schema declaration
the external policy MAY follow to grandfather grants (never self-granting). Registration = routing +
namespace-claim, not the security root. The authz-collision thread is CLOSED; the model is set (I5
build deferred to the operator's return, but no open design question remains).

## Part C — stateful-resource transport primitives: process / ws-client / ws-server (I12–I14)

> Operator capstone: *"the same thing could be done with a websocket client connection … and the same
> goes for a server! … anything stateful/kernel side looks like any other effect handler."*

These are the minimal, stateful, transport-level byte-movers the kernel/host DOES own — each presented
to userspace as a handler session with a defined effect contract. They are siblings of the landed `ws/*`
family (#2807): opaque-frame streaming over a stateful connection, an outbound send-shaped effect, and
connect/disconnect-shaped inbound lifecycle events. `v-agent-harness-host` owns the host mechanisms
(confirmed feasible — reuses the `ws_socket` streaming Send/!Send split + `shell.rs` spawn-safety); the
`proc/*` kernel family vocab is `v-agent-harness`'s lane, shaped exactly like `ws/*`.

### I12 — local PROCESS session (spawn/stdio-as-frames/stdin/kill/exit)

> Operator: *"create a session from a local process … effects would be stdout and stderr … the thing
> that spawns it would get notifications … could submit inbound stdin frames or kill it … exactly what
> would allow spawning and monitoring a local MCP server … minimal and generic."*

A generic host primitive: spawn a local process as a managed child, modeled as a session.

- **Shape (mirrors the `ws/*` family, NOT `shell.rs`'s one-shot `output()`):** a long-lived managed
  child with STREAMING stdio. On spawn the host mints an opaque **proc-id** (like a ws conn-id), spawns
  a `tokio` child, and launches reader tasks that pump stdout/stderr chunks INCREMENTALLY into the
  loop's `Inbox` as `Inbound` frames tagged with the proc-id (the ws_socket `Send`/`!Send` split
  verbatim). The spawner (a userspace session) receives them as `Inbound` events.
- **Effect/event vocab (ws-analogous):** `proc/spawn` (target = command + args + env; result = proc-id
  — reuses `shell.rs` `Command::new(program).args` spawn-safety, NO `sh -c`, CWE-78-safe + the Cedar
  gate, but NOT `output()`); `proc/stdin` (outbound, target = proc-id, payload = bytes — `ws/send`-shape);
  `proc/kill` (outbound, target = proc-id); inbound `proc-stdout` / `proc-stderr` frames and `proc/exit`
  (payload = exit status — the `ws/disconnect`-shape lifecycle event). `proc/spawn`↔`proc/exit` are the
  connect/disconnect analogs.
- **Feature-gated** (`live-exec` alongside the shell executor, or a new `live-proc`). This is the
  mechanism a userspace MCP handler uses to spawn + drive a local MCP server (JSON-RPC over the child's
  stdin/stdout) with ZERO MCP in the host — the outpost reframe made concrete.

*Gate:* a host test — spawn a process, receive its stdout as `Inbound` proc-stdout frames, submit
stdin, observe `proc/exit` with the status; kill terminates it. **Anchors:** new
`cdz-agent-host/src/proc_session.rs` (streaming child + reader tasks, mirrors `ws_socket.rs`),
`effect.rs::effect_ct` (`PROC_PREFIX` + `proc/spawn`/`proc/stdin`/`proc/kill` + `proc/exit` inbound,
shaped like `ws/*`), `shell.rs` (spawn-safety reuse).

### I13 — WS CLIENT session, and I14 — WS SERVER session

> Operator: *"spawn a session and it is actually a [websocket] client that you can interact with in a
> defined contract … the same goes for a server! You could spawn a websocket server and listen on a
> specific port as part of genesis and it would execute effects routed via the same mechanisms."*

The same stateful-resource-session pattern for network sockets, completing the family:

- **I13 WS CLIENT:** spawn a session that IS an outbound websocket client connection; interact via a
  defined effect contract (send/recv frames, close). The existing `ws/*` family is server-inbound
  (peers connect to the host, #2807); this is the client-outbound dual — the host dials out, mints a
  conn-id, and the spawning session drives it with `ws/send` + folds inbound frames. Reuses the ws
  transport wholesale (opposite dial direction).
- **I14 WS SERVER:** spawn a websocket server listening on a port (e.g. as part of genesis); incoming
  connections + frames arrive as the existing `ws/connect` / `ws/disconnect` / inbound-frame events,
  routed to the owning handler session via the same machinery. This is essentially THE OUTPOST O1 host
  half (already built) re-described as a spawnable stateful-resource session rather than a bespoke host
  mode — the reframe unifies it.

⟨D12⟩ *default:* Part C ships AFTER the Part A critical path (v-ah-host will slot I12 with the outpost
O1b streaming work since it is the same shape); I13/I14 are largely a re-description + client-dual of
the landed `ws/*` transport, so they are small once I12's pattern is set. *Gate:* per primitive — spawn,
exchange frames via effects, receive lifecycle notifications, terminate. **Anchors:** `ws_socket.rs`
(reuse/extend for client dial + server spawn), the `ws/*` family (#2807).

## The outpost as a bootstrap-reducer application (no new mechanism)

> Operator: *"the outpost really is just executing sessions … one reducer that tries to connect to the
> central hub and download the current set of reducers … then spin all of that up and handle effects
> from the local machine."* And the closing detail: *"all an outpost would be is a single config entry
> saying where to bootstrap from. And that is basically the only difference."*

The outpost needs NO bespoke design — it is a pure composition of primitives already in Parts A and C.
Concretely, an outpost is a plain host whose genesis runs a **bootstrap reducer** that:

1. **Connects to the central hub** — over the WS-CLIENT session primitive (I13).
2. **Downloads the current reducer set** this outpost needs — the reducer wasm components are
   content-addressed, fetched via the blob/CAS + `store/resolve` seams (Part A + existing blob/store
   families).
3. **Spins them up as child sessions** — via `lifecycle/spawn` (each downloaded reducer becomes a
   handler session, wasm or the host-native kind per the layer model above).
4. **Handles effects from the local machine** — those spawned sessions serve the local effect families
   (`shell` / `fs` / `proc` / `ws` + any userspace handlers), exactly as any handler session does.

So the outpost = **a plain host + ONE config value**: the bootstrap source (the hub/bootstrap address).
Same binary, same primitives, no separate mode, no bespoke mechanism — the only thing that makes a host
an "outpost" is where its bootstrap reducer points. On startup the host reads that config entry and the
bootstrap reducer does the four steps above. This closes the outpost story: it is fully a config-driven
guest bootstrap over the existing primitives, and it re-confirms that MCP + federation are pure
CONSUMERS of this model (a downloaded userspace handler), never host code. `v-agent-harness-host` owns
the outpost re-framing against its O1 host half (already built) + the I12/I13 transport primitives.

## The gate that protects it

- Kernel: `cargo test -p rcdzc --lib` — the I1 registration/resolution unit, the I2 deferred-settlement
  + replay unit. `cargo xtask gate` fail-set additive-only.
- Host: `cargo test -p cdz-agent-host` — the I3 forward-framing unit, the I4 round-trip E2E (+ forged/
  stale/double-token refusal), the I5 Cedar coarse-gate E2E, the I6 terminate-prune unit.
- `cargo xtask check` clean (fmt + clippy -D + codegen --check). No `cdz-runtime` `//`/`wit` edits
  (REQUIRED_RUNTIME_HASH frozen).
- The invariant a fuzzer/breaker probe should target once landed: a caller can NEVER cause an effect to
  execute under the handler's grants except through the registered narrow family — i.e. attenuation is
  not bypassable by any request shape (the confused-deputy property).

## Open decisions ⟨operator may ratify⟩ — each with a chosen default so the build is unblocked

- **D1 — registration namespace.** `effect/<family>` as a NEW top-level write-authority prefix, vs.
  nesting under `system/effect/<family>`. *Default:* a dedicated `effect/` prefix, authz-gated exactly
  like `system/` — it is a distinct authority (who may publish platform effects) worth naming
  first-class, and keeps `system/` for the runtime's own pointers.

- **D2 — reply mechanism.** A dedicated `effect/reply` family the handler emits (chosen), vs. reusing
  `Emit` back to the caller (rejected — `Emit` delivers an Inbound, it does NOT settle a pending effect
  by id, so the caller would never resume its `weather` continuation), vs. an implicit "the handler's
  fold return value IS the reply" (rejected — a reducer produces effects, not a return value, and a
  handler may need multiple ticks / its own effects before it can answer). *Default:* explicit
  `effect/reply`, token-authorized (possession of the host-minted reply-token = authority).

- **D3 — one handler per family, or many.** Single-value pointer `effect/<family> → SessionId`
  (chosen default — the degenerate case, matches the date example) vs. an OR-set group of handlers for
  failover/load-balance (the multi-value session-directory case). *Default:* single pointer; a group is
  purely additive later (resolve-all → pick one), no protocol change.

- **D4 — one reply per request, or streaming.** *Default:* exactly one reply per request (one-shot
  settlement by `EffectId`, the cleanest and matches every existing effect). Streaming/multi-response
  is a later additive (the handler emits Inbound events to the caller alongside the one settling reply),
  not v0.

- **D5 — is the coarse Cedar gate mandatory, or may a family be handler-authorized only?** *Default:*
  the coarse kernel/Cedar gate on `(family, target)` is MANDATORY and deny-by-default (a caller with no
  grant for the family never reaches the handler); the handler's fine-grained authz is optional and
  additive. This keeps the security boundary in the kernel/policy, not delegated wholesale to guest code.

- **D6 — handler-calls-handler re-entrancy / cycles.** *Default:* allowed (a handler may emit userspace
  effects served by other handlers; the cause-DAG audits the chain). Cycle/`depth` protection is
  deferred — a looping handler is a handler bug bounded by the effect-id budget + timeouts, not a kernel
  concern in v0. Flag for the operator in case they want a hard depth ceiling.

- **D7 — the handler's INTERFACE descriptor.** PROMOTED to first-class by the operator's introspection
  extension (I11): a handler publishes a machine-readable contract (ops/target-schema + docs + type
  signatures) under its `effect/<family>` name, in binary AST. *Default:* the descriptor is a binary-AST
  artifact reusing the `control/signature` + cadenza-docs encoders; discovery is the I11 `control/effects`
  query. Not required to ship the Part A request/response loop (Part A works with opaque payloads), but
  it is the input to I11 introspection.

- **D8–D12 — Part B/C decisions** are stated inline in each increment (D8 single-overlay-per-session
  runtime chain; D9 role attach-order precedence; D10 per-family middleware stack; D11 introspection
  returns the RESOLVED reachable set as binary AST; D12 Part C ships after the Part A critical path).
  Each has a chosen default so the build is unblocked; flag any the operator wants to re-decide.

- **⟨operator asks worth surfacing⟩** given the cascade: (a) the kernel/userspace TRANSPORT line — the
  operator ruled "kernel = transport-level byte-movers only (ws/stdin/tcp/http); anything modelable over
  multiple transports (JSON-RPC, MCP) is userspace" — captured in the capstone; confirm no
  application-protocol logic creeps into the host. (b) The scope: this doc now spans Part A (foundation,
  building) + Part B (overlays/roles/middleware/introspection) + Part C (process/ws-client/ws-server
  transports). Is this the coherent whole to hand the PM, or should Parts B/C split into follow-on
  design docs once Part A lands? *Default:* keep as one coherent doc (the operator asked to "keep it one
  coherent doc"), build in the A→B→C order, each part independently landable.

## Why this is foundational (and safe to build incrementally)

This reshapes how capabilities/authz work platform-wide in the good direction the operator intends: the
DANGEROUS primitives (`shell`, raw `http`, credentials) get wrapped once, in audited handler sessions,
and every other session holds only cheap narrow grants over virtual resources. It is the runtime,
stateful realization of §20a + §12f + §20b already on the books — so it is not a speculative new model,
it is the mechanism those decided sections were waiting for. And Part A lands on existing seams: the
ONLY new kernel primitive is I2 (deferred settlement, itself a generalization of code already present
for `control/signature`); registration is a name-server write, routing is a host executor, delivery
reuses `Emit`/inbound, correlation reuses `EffectId`, attenuation reuses the per-session authz boundary.

The full arc (Parts A–C) is one idea — the capstone: **anything stateful/kernel-side is just another
effect handler.** Userspace effects, capability attenuation, overlays, roles, middleware, introspection,
local processes, and websocket client/server sessions are all the SAME machinery: spawn a handler,
interact via effects, get lifecycle notifications, terminate. The kernel keeps only the minimal stateful
byte-movers (process stdio, ws transport); every application concern — including JSON-RPC and MCP —
becomes a userspace handler, so THE OUTPOST needs no MCP in the host at all. The kernel stays minimal;
the platform becomes extensible in userspace — the fullest expression of the minimize-kernel thesis.
Each part is independently landable (A is being built now against a ratified I1/I2 shape; B and C are
additive and do not disturb it), so the whole is safe to grow incrementally.
