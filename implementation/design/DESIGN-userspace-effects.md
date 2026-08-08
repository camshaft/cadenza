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
`design-dogwood-cedar` (the authz resource/entity model the handler registers into), and `v-effects`
(effect semantics). Overlaps THE OUTPOST (`ws/*` gateway) and the minimize-kernel theme.

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

## Increments (top-to-bottom, the way a vertical lands them)

Each increment is independently green and gated. I1–I2 are `cdz-kernel` (v-agent-harness); I3–I4 are
`cdz-agent-host` (v-agent-harness-host); I5 is authz (with design-dogwood-cedar); I6 is lifecycle.

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

- **D7 — the handler's INTERFACE descriptor.** Should a handler publish a machine-readable descriptor
  of the ops/target-schema it accepts (so callers discover the interface, like `control/signature` for
  components)? *Default:* out of v0 scope but designed-compatible — a handler MAY register a
  `control/signature`-style descriptor under its `effect/<family>` name; discovery reuses the existing
  signature-query path. Not required to ship the request/response loop.

## Why this is foundational (and safe to build incrementally)

This reshapes how capabilities/authz work platform-wide in the good direction the operator intends: the
DANGEROUS primitives (`shell`, raw `http`, credentials) get wrapped once, in audited handler sessions,
and every other session holds only cheap narrow grants over virtual resources. It is the runtime,
stateful realization of §20a + §12f + §20b already on the books — so it is not a speculative new model,
it is the mechanism those decided sections were waiting for. And it lands on existing seams: the ONLY
new kernel primitive is I2 (deferred settlement, itself a generalization of code already present for
`control/signature`); registration is a name-server write, routing is a host executor, delivery reuses
`Emit`/inbound, correlation reuses `EffectId`, attenuation reuses the per-session authz boundary. The
kernel stays minimal; the platform becomes extensible in userspace — exactly the minimize-kernel thesis.
