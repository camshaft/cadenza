# DESIGN — the system reducer as a Cadenza guest (map of contracts + default event handler)

**Owner:** v-platform (system-reducer + capability/dispatch model, `design/cadenza-platform.md` §3/§4/§5).
**Status:** proposed — operator-directed rework, pending a build-vs-refine steer.

## The operator's direction

Reviewing the §9 checker PR (#3157), the operator asked (verbatim):

> "How is there no system reducer? Speaking of, we should really rework that to be a map of contracts with
> a default event handler. But reducers should not be able to execute the deliver contract — they need to
> emit an effect which is handled by the event reducer who executes it on their behalf."

Three things: (1) why does a run show "no system reducer"; (2) the system reducer should be a **map of
contracts with a default event handler**; (3) an ordinary reducer must **not execute deliver** — it emits an
effect, and the **event reducer** executes deliver on its behalf.

## What already holds (so the rework is smaller than it looks)

- **The map+default already exists.** `EventRegistry` (`event_registry.rs`) IS "a map of contracts with a
  default event handler": `resolve(contract) -> ProgramHash` always yields a program — an installed override,
  else the default. `InMemoryEventRegistry { default, overrides }`. This is the design of record for "which
  program governs a contract's events."
- **The capability boundary is already enforced AND tested.** `ReducerKind::Ordinary`'s deliver is ignored;
  only a `ReducerKind::Event` reducer's deliver is carried out (`system.rs`; tests
  `an_ordinary_reducers_deliver_is_not_honored`, `the_system_routes_an_ordinary_effect_to_the_system_reducer_for_its_contract`).
  The `deliver` host import is wired only on `event-reducer-world`, never on `reducer-world` — so the world a
  guest is compiled against, plus its kind, is the gate.
- **Dispatch already routes for the emitter.** When an ordinary reducer emits a Request on a contract, the
  kernel resolves the contract to a handler program via the registry and spawns it as a privileged `Event`
  reducer, per-event, delivering the effect to it stamped with the emitter's origin (`system.rs`).

So constraint (3) is already the architecture, and (2)'s abstraction already exists. **"No system reducer"**
in #3157 is because (a) the conformance harness reads a guest's emitted effect straight from the observation
log (record-and-assert) rather than standing up a routing substrate, and (b) there is **no concrete default
event handler as a real reducer program** — the registry's default is an abstract `ProgramHash`, with nothing
behind it.

## The proposal — make the default event handler a Cadenza guest

Build **the system reducer as a first-class Cadenza event-reducer guest** = the registry's **default event
handler**, fitting the every-guest-in-Cadenza directive. On each effect (a `Message` the kernel routes to it):

1. Read the routing substrate — the handler chain for the effect's contract — from the reducer graph via
   `graph.neighbors(contract/target) -> list<list<u8>>` (v-rb's shape-e, in flight). "Map of contracts" =
   the registry's overrides; "chain" = the graph edges the neighbors read returns.
2. If a handler is registered for the contract, **forward** the effect to it (deliver it into the handler's
   log). Otherwise apply the **default** behavior. Both are the event reducer executing `deliver` on the
   emitter's behalf — the emitter only ever emitted an effect.
3. On a handler's reply, **respond** back to the caller (deliver a `response` correlated by token) — the
   answer-back path the §4 dispatch trilogy guests already demonstrate.

The §4 dispatch guests (notification/message/response, merged #3164/#3180/#3181) are the primitives this guest
composes: it IS an event reducer performing `deliver`, now driven by a graph read rather than echoing to the
sender. `forward`/`respond` are the event reducer's dispatch contracts (not kernel built-ins).

## Capability model (the sharp part)

- **`deliver` stays event-reducer-only.** No change: the world split already gates it. Ordinary reducers emit
  effects; the system reducer (an `Event` reducer) delivers. Keep the `ReducerKind::Ordinary`-deliver-ignored
  test as the guard.
- **`cas-pin`/`cas-unpin` — OPEN QUESTION this rework must settle.** The CAS-GC design (`DESIGN-cas-pinning-gc.md`)
  made pins **direct capability-gated host calls** (a reducer touches the store mid-fold; routing every touch
  through the effect model is clumsy). The operator's "reducers emit effects the event reducer handles on
  their behalf" model raises the alternative: **pin as an effect** routed to the system/GC reducer. Two options:
  - **(A) Direct gated host call** (design's current): `cas-pin(hash)` is a host call on the reducer's world,
    auto-scoped to the session, allowed only where the capability gate permits. Ergonomic; keeps pinning
    synchronous ("this is now kept"). The gate is the host-call authorization surface.
  - **(B) Effect routed to the GC/system reducer**: a reducer emits a pin *effect*; the GC reducer folds it
    and records the pin. Uniform with the deliver model; more governable; but asynchronous-with-reply where a
    reducer wants a synchronous keep, and adds ceremony to a routine store touch.
  - **Recommendation:** (A) for `cas-pin`/`cas-unpin` (they are store operations, not routing), with the
    capability GATE explicit — a reducer holds the pin capability only if granted, mirroring how `deliver` is
    world-gated. This is why **CAS-GC increment 4 (pin host calls) is paused** pending this decision; increments
    1–3 (store surface, ledger liveness, collect — PRs #3175/#3177/#3178) are pure logic and independent.

## Increments (a vertical v-platform owns)

1. **Default-system-reducer guest (skeleton).** A Cadenza event-reducer guest on `event-reducer-world` that,
   on a message, reads its own contract's handlers via `graph.neighbors` and forwards/handles-by-default.
   *Gated on v-rb's `graph.neighbors` (shape-e).*
2. **Wire it as the registry default.** The `EventRegistry`'s default program becomes this guest's program
   hash (built + content-addressed like any guest). A conformance run stands up the system reducer (not the
   read-verdict-from-log shortcut) and asserts an ordinary reducer's effect is routed + answered.
3. **Capability gate hardening + `set-edges` (shape-f).** The system reducer manages the substrate (install a
   handler = set a graph edge); `set-edges` is the write half after `neighbors` is the read half.
4. **Resolve `cas-pin` capability (A vs B), then resume CAS-GC 4–6** under the settled model.

## Coordination

- **v-rb:** `graph.neighbors` (shape-e, read — confirmed my next need) then `set-edges` (shape-f, write). The
  `forward`/`respond` dispatch may need codegen surface too.
- **v-platform-itest:** conformance runs that stand up the system reducer and assert routing/answer-back — and
  that are **non-vacuous** (the CAS must seed the runtime, #3184; a guest that silently fails to instantiate
  passes spawn-only exit 0 and validates nothing).

## Open questions for the operator

1. Design-note-first (this) vs. build increment 1 directly? (The build is gated on `graph.neighbors` regardless.)
2. `cas-pin` capability: direct gated host call (A, recommended) vs. routed effect (B)?
3. Is the default event handler's default behavior "reject / no-handler error" for an unregistered contract, or
   a permissive forward? (Affects the `MissingHandler` error semantics in §4.)
