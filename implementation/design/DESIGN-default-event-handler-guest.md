# DESIGN — the default event handler as a Cadenza guest + a settable event registry

**Owner:** v-platform (event-registry + capability/dispatch model, `design/cadenza-platform.md` §3/§4/§5).
**Status:** proposed — operator-directed rework, pending build steer.

> Terminology (per operator feedback): this note uses the **event registry** (the map contract → handler,
> with a default) and the **default event handler** (the program that governs a contract with no override). It
> deliberately avoids the "system reducer" phrase, which the operator found confusing — the code comments that
> still say "system reducer" (system.rs, graph.rs, event_registry.rs, …) are a follow-up terminology cleanup to
> the same vocabulary.

## The operator's direction

Reviewing the §9 checker PR (#3157), the operator asked (verbatim):

> "How is there no system reducer? Speaking of, we should really rework that to be a map of contracts with a
> default event handler. But reducers should not be able to execute the deliver contract — they need to emit
> an effect which is handled by the event reducer who executes it on their behalf."

and, reviewing this note (#3185):

> "I know the platform is implemented as a registry of events. But the integration harness doesn't expose the
> ability to set those, right? It's also still using the system terminology which is confusing."

So: (1) the event handler for a contract is a **map + default**; (2) an ordinary reducer must **not execute
deliver** — it emits an effect the **event reducer** carries out on its behalf; (3) the integration harness
must be able to **set the event registry** (default + per-contract overrides), which it cannot today.

## What already holds

- **The map+default already exists.** `EventRegistry` (`event_registry.rs`) IS the map contract → handler with
  a default: `resolve(contract) -> ProgramHash` always yields a program — an installed override, else the
  default. `InMemoryEventRegistry { default, overrides }`, `set_override(contract, program)`.
- **The capability boundary is already enforced AND tested.** `ReducerKind::Ordinary`'s deliver is ignored;
  only a `ReducerKind::Event` reducer's deliver is carried out (`system.rs`; tests
  `an_ordinary_reducers_deliver_is_not_honored`, `the_system_routes_an_ordinary_effect_to_the_event_handler_for_its_contract`).
  `deliver` is a host import wired only on `event-reducer-world`, never on `reducer-world` — the world a guest
  compiles against, plus its kind, is the gate.
- **Dispatch already routes for the emitter.** When an ordinary reducer emits a Request on a contract, the
  kernel resolves the contract via the registry and spawns that program as a privileged `Event` reducer,
  per-event, delivering the effect to it stamped with the emitter's origin.

So constraint (2) is already the architecture and (1)'s abstraction already exists. What is missing is (i) a
concrete **default event handler as a real reducer program** (the registry's default is an abstract
`ProgramHash` with nothing behind it — this is why a run shows "no [default] event handler"), and (ii) a way
to **set the registry from the harness**.

## Gap A — the harness cannot set the event registry

The integration harness (`HarnessSpec`, `testing/spec.rs`; itest binary) can spawn reducers, deliver messages,
and run pure programs — but it does **not** expose installing event-registry entries. So a conformance run
cannot say "route contract C's effects to handler H" or "the default handler is program P"; every run uses
whatever default the binary hard-wires. The operator wants this settable.

**Proposal:** add a `HarnessSpec` directive to populate the registry before the run:

```
registry {
  default = <program-name>          # the default event handler (a guest program by name)
  handler { contract = <id>; program = <program-name> }   # 0+ per-contract overrides
}
```

The itest binary builds an `EventRegistry` from it (`InMemoryEventRegistry::new(default)` + `set_override`
per handler) and hands it to the system. This is v-platform-itest's harness surface + v-platform's registry;
**coordinate with v-platform-itest** (a spec-directive + binary-wiring slice, sibling to their pure-run
directive #3179). It also makes the default-event-handler conformance run below expressible.

## Gap B — the default event handler is not yet a Cadenza guest

Build **the default event handler as a Cadenza event-reducer guest**, fitting the every-guest-in-Cadenza
directive. On each effect (a `Message` the kernel routes to it):

1. Read the handler chain for the effect's contract from the reducer graph via `graph.neighbors(node, kind,
   dir)`. The exact call is now pinned to the code's convention (see **the graph convention** below): the edge
   kind IS the contract-id, and the source node is a reducer that owns a chain for that contract.
2. If a handler is registered, **forward** the effect to it (`deliver-message` into the handler's log);
   otherwise **handle by default** — an unregistered contract is NOT auto-rejected by the routing layer; this
   guest (the default handler) receives it and decides (operator decision below). Both are the event reducer
   executing `deliver` on the emitter's behalf — the emitter only ever emitted an effect.
3. On a handler's reply, **respond** back to the caller (`deliver-response` correlated by `token`). The default
   handler, if it declines an effect, sends a **rejection response** (`answer = Err(missing-handler)`) back to
   the caller the same way.

The §4 dispatch guests (notification/message/response, merged #3164/#3180/#3181) are the primitives it
composes: it IS an event reducer performing `deliver`, driven by a graph read rather than echoing to the
sender.

### The graph convention (what the code actually holds — corrected)

An earlier draft said "`graph.neighbors(contract/target)`", which was imprecise. The reducer graph
(`graph.rs`) models a handler chain as **`owner_reducer -> handler` edges keyed by the contract-id as the edge
kind**: `EdgeKind::for_contract(contract) == contract.hash()` (`graph.rs`), and the Rust convenience
`ReducerGraph::resolve(reducer, contract)` reads exactly `neighbors(reducer, for_contract(contract),
Dir::Out)` in weight (chain) order. So a chain is keyed on a **(reducer, contract) pair** — the *node* is a
reducer-id, the *kind* is the contract-id.

This is friendly to a guest: in the message envelope (`reducer.wit`), `contract-id`, `reducer-id`, and the
graph's `edge-kind` are all `list<u8>`, and `EdgeKind::for_contract` is the contract-id's bytes verbatim — so
the guest passes `msg.contract` **directly** as the `kind` argument (no derivation), and a `reducer-id` (e.g.
`msg.sender.reducer`) as the `node`. The reject path builds a `response { contract = msg.contract, token =
msg.token, answer = Err(missing-handler) }` and `deliver-response`s it to `msg.sender.reducer`.

**Current runtime reality (important).** Handler chains in the graph are **API + unit-test surface only**:
`for_contract` / `set_chain` / `ReducerGraph::resolve` are referenced nowhere but `graph.rs` and its tests. The
kernel installs only structural edges (spawn tree, `watch-exit`), and **live dispatch does not read the graph
at all** — it resolves a contract through the Rust `EventRegistry` (`events.resolve(contract) -> ProgramHash`,
`system.rs`), which is the sole routing mechanism today and maps a contract to *which program* is the event
reducer. There is **no bridge** between EventRegistry entries and graph chain edges. So the two mechanisms are
distinct: the EventRegistry picks the program (this guest, for a contract with no override); this guest, once
built, reads the *graph* for finer per-owner delegation. Whether they coexist that way or dispatch should
migrate to a graph read is an open question (below) the operator should settle before increment 3 wires it.

## Capability model

- **`deliver` stays event-reducer-only.** No change: the world split gates it. Ordinary reducers emit effects;
  the event handler (an `Event` reducer) delivers. Keep the `Ordinary`-deliver-ignored test as the guard.
- **`cas-pin`/`cas-unpin` — DEFERRED.** The operator is reviewing the CAS-GC direction and asked to hold, so
  this note does **not** resolve whether pins are a direct capability-gated host call or a routed effect. Left
  open until the operator's CAS-GC review; CAS-GC increment 4 (pin host calls) stays paused regardless.

## Increments (a vertical v-platform owns)

1. **Harness-settable registry (Gap A).** The `registry` `HarnessSpec` directive + itest-binary wiring. Not
   gated on anything; coordinate with v-platform-itest. Unblocks expressing the run in increment 3.
2. **Default-event-handler guest (Gap B, skeleton).** A Cadenza event-reducer guest that, on a message, reads
   its contract's handlers via `graph.neighbors(node, msg.contract, outgoing)` and forwards (`deliver-message`)
   / handles-by-default (empty chain → `deliver-response` with `Err(missing-handler)`). `graph.neighbors`
   emit+load landed (#3200), **but a Cadenza-guest INVOKE of it is not yet proven** (v-rb did emit+load; the
   lift path is proven by the kv.get / prefix-scan invokes, not a `neighbors` invoke) — so the first build step
   is to prove a real guest can call `neighbors` and fold its `list<reducer-id>` result. Also blocked on the
   `node` convention (open question 3). *Gated on: that invoke proof + the node decision.*
3. **Wire it as the registry default + a non-vacuous conformance run.** The registry default becomes this
   guest's program; a run installs it via the Gap-A directive, spawns it, emits an ordinary effect, and
   asserts it is routed + answered — **confirmed non-vacuous** (the CAS must seed the runtime, #3184; a guest
   that silently fails to instantiate passes spawn-only exit 0 and validates nothing).
4. **`set-edges` (shape-f) + capability hardening.** The event handler manages the substrate (install a
   handler = set a graph edge); the write half after `neighbors` is the read half.

## Coordination

- **v-rb:** `graph.neighbors` emit+load landed (#3200); the next need is a proven Cadenza-guest **invoke** of
  it (fold a real `list<reducer-id>` result), then `set-edges` (shape-f, write).
- **v-platform-itest:** the Gap-A `registry` directive + non-vacuous conformance runs (#3184's CDZ_STORE seed);
  and, depending on open question 4, whether the `registry` directive also seeds graph chain edges.

## Observation-model consideration — is the deliver act recorded?

Surfaced building the §4 dispatch conformance runs (v-platform-itest): an event reducer's `deliver` is a host
call (`host.rs` `deliver_notification` → `System::deliver` → the target's mailbox) — it writes **nothing** to
the observation log, and it is not a `step.requests` entry so it is not recorded as `Emitted`. A `Delivered`
appears **only when a spawned target folds it** (its recording wrapper). So a routed event to an unspawned
sender records nothing, and "spawn an event reducer, deliver a message, assert what it routed" is not
assertable without a spawned listener to receive it.

For the §4 runs today the fix is a harness one (v-platform-itest): deliver with `from = { task = <spawned
listener> }` so the routed event lands on a spawned reducer that records the `Delivered` (the run tests the
full route). But it raises a real question for this rework: **should the platform record the deliver act
directly** — the event handler's whole purpose is to route, and that act being invisible unless the target is
spawned means the conformance suite can't observe routing behavior in isolation. Options: leave it (routing is
observed via the target's `Delivered`), or add a record for the deliver act at the host boundary. Deferred to
this rework rather than changed piecemeal — it belongs with the default-event-handler model, since that guest's
routing is exactly what we'd want observable.

**Resolved (record the deliver act — via the injectable delivery factory).** #3197 gives the store a
per-reducer injection seam for each node-wide host capability: `graph`/`program-of`/`deliver` are built by
factories (`GraphFactory`/`ProvenanceFactory`/`DeliveryFactory` = `Fn(ReducerId) -> Arc<dyn …>`), uniform with
the `make_blobs`/`make_kv` factories that already build a reducer's `blobs`/`state`. The store does not know
what a factory returns — it is a general seam (#3199), and recording is one use of it. So the harness supplies
a `DeliveryFactory` that builds a recording decorator over the base delivery capability: it records the deliver
act at the host boundary — which primitive (`deliver-{notification,message,response}`), which
contract/token/payload — the moment the event reducer calls it, independent of whether any target is spawned.
Routing is thus observable in isolation. Two consequences agreed with v-platform-itest:

- The §4 dispatch conformance runs assert on the recorded `deliver-{notification,message,response}` `Entry`, so
  they **retire the `from = { task = <listener> }` idiom**: spawn the event reducer, deliver to it, assert the
  recorded deliver act. `from-by-task-name` is built later only if a sender-sensitive-reducer test needs a
  realistic spawned sender (it tests full-route landing + sender sensitivity, which §4 dispatch does not need).
- Non-vacuity bar: a run asserts the **specific** recorded `Entry` (primitive + contract + token + payload) the
  guest produced, not merely "a deliver entry exists" — a guest that silently fails to instantiate must not
  pass. This keeps the deliver-recorder run as non-vacuous as the old listener idiom.

The recording lives in the harness (v-platform-itest), not the kernel: the decorator is what a
`DeliveryFactory` builds, so the platform stays free of any observation-log dependency — the store just calls
the factory it was given (#3197/#3199).

## Operator decisions (resolved)

- **Build the harness-settable registry (increment 1): YES.** Operator: "we should build the change to be able
  to specify the registry." It is non-gated (v-platform-itest's lane); the default-event-handler guest
  (increment 2) remains gated on `graph.neighbors`.
- **Default no-handler semantics: FORWARD, don't auto-reject.** Operator: "If a handler isn't registered in the
  registry then it's forwarded to the default handler, which then decides what to do with it. If it doesn't
  like it then it rejects and sends a rejection response back to the caller." So the registry does not produce
  `MissingHandler` for an unregistered contract — it forwards to the default handler, whose own logic may reject
  (a rejection response to the caller). `MissingHandler` becomes the default handler's decision, not a routing
  auto-fault.

## Open questions

1. ~~Should the platform record the deliver **act** at the host boundary?~~ **Resolved** — yes, via a recording
   decorator the harness supplies through #3197's injectable `DeliveryFactory` seam (see the observation-model
   consideration above); `from-by-task-name` is retired for §4 dispatch.
2. (`cas-pin` capability — direct gated host call vs. routed effect — is deferred to the operator's CAS-GC
   review; CAS-GC increment 4 is paused on it.)
3. **What `node` does the default handler read the chain from?** The graph keys a chain on `(owner_reducer,
   contract)`. For a *global* default handler two readings are possible: (a) the **emitting reducer** —
   `neighbors(msg.sender.reducer, msg.contract, outgoing)` — which is the §5 owner-middleware model (a
   reducer's own installed chain, walked up its ancestors); or (b) a **well-known root node** the registry
   installs global handlers under, so the default handler looks up a single global chain per contract. These
   are different semantics; (a) matches the graph's existing `resolve(reducer, contract)` convenience, (b)
   matches a registry-like "one handler per contract" model. *Needs the operator.*
4. **Do the EventRegistry and the graph chain coexist, or does dispatch migrate to a graph read?** Today
   dispatch reads only the EventRegistry (contract → program); graph chains are unused at runtime. Increment 3
   ("wire it as the registry default") implies coexistence — registry picks the program, this guest reads the
   graph for delegation — but the operator may instead want dispatch itself to route via the graph. This
   decides who installs chain edges (increment 4 `set-edges`, and whether the Gap-A harness `registry`
   directive also seeds graph edges). *Needs the operator.*
