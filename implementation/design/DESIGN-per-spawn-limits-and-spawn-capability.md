# DESIGN — per-spawn resource limits, a spawn WIT capability, and the privileged-spawn / non-privileged-effects split

**Owner:** v-platform (capability + dispatch model, `design/cadenza-platform.md` §3/§4/§5).
**Status:** settled + approved — the operator answered all five questions (Q1 spawn handler = an ordinary
event reducer; Q2 host-side clamp = yes; Q3 one coarse privileged world; Q4 a request may set `links` but NOT
`kind`; Q5 per-node = default + ceiling). Cleared to merge. Sequencing greenlit; builds on the per-node
`ResourceLimits` of #3209.

## The operator's direction

Extending the resource-limit work (#3209, per-node `ResourceLimits` from config — the foundation), the operator
directed (verbatim):

> "i think we should have per-spawn limits on memory and max yields as well. and then the event handler for
> that would be able to decide if those limits are ok. so then we'd need to have a WIT for spawn and then
> effects would be for non-priv reducers."

Four moves, each building on the last:

1. **Per-spawn limits** — a spawn carries its own memory + max-yields limits, not just the node's.
2. **Admission control** — a *spawn handler* inspects a spawn's requested limits and decides accept/reject.
3. **A spawn WIT** — spawn becomes a typed, capability-gated host import, not an ambient act.
4. **Privilege split** — spawn is the *privileged* path (via the spawn WIT); *effects* are the non-privileged
   floor. Privileged reducers spawn (and admit limits); non-privileged reducers only emit effects.

## What already holds (the ground this builds on)

- **`ResourceLimits` is per-node config (#3209).** `{ epoch_tick, yield_every, max_yields,
  max_linear_memory_bytes }`, threaded from node assembly into `arm_store_safety`, which arms every reducer
  store with the compute (epoch yield+trap) and memory (linear-memory ceiling) bounds. Today it is uniform
  across every reducer on the node.
- **A `Spawn` request already exists.** `system.rs` `Spawn { id, program, nonce, parent, kind, links }` — the
  kernel launches a reducer from it (§7). `SpawnContext { id, kind }` reaches `ProgramStore::spawn`, and the
  `kind` already fixes the capability set.
- **The world split is the capability mechanism.** `deliver` is a host import wired only on
  `event-reducer-world`, never `reducer-world` — the world a guest compiles against, plus its `ReducerKind`,
  *is* the capability gate (no runtime check). Spawn-as-a-WIT reuses exactly this pattern.
- **Dispatch already routes an effect to a handler.** An ordinary reducer emits a Request; the kernel resolves
  the contract and spawns the event reducer that carries it out (§4). The *spawn handler* is the direct analog
  for spawn requests.

## The design

### 1. Per-spawn limits — `SpawnLimits` on the `Spawn` request

Add the requested per-spawn bounds to the spawn:

```
struct SpawnLimits { max_linear_memory_bytes: usize, max_yields: u64 }   // the two the operator named
Spawn { …, limits: SpawnLimits }
```

`epoch_tick` and `yield_every` stay node-wide (the ticker cadence is a property of the node's runtime, not of
one reducer). Only the two *budgets* the operator named are per-spawn.

**Resolution against the node (Q2/Q5, operator-settled).** The per-node `ResourceLimits` (#3209) is the
**default and the ceiling** (Q5). The effective limits a store is armed with = the spawn's requested limits,
**clamped to the node ceiling** (a spawn can never request *more* than the node permits) — but admission
(below) can reject a spawn before it even reaches clamping, so the two compose: the admission reducer is the
policy, and the host-side clamp is a hard backstop *underneath* it (Q2, operator: "it's fine to have an
additional clamp from the host" — defense in depth, so a buggy or absent admission reducer still cannot grant
more than the node permits). The resolved `SpawnLimits` ride on `SpawnContext` into `arm_store_safety`, so a
store is armed with *this reducer's* budget
instead of the uniform node value.

### 2. Admission control — an ordinary event reducer (Q1, operator-settled)

**A spawn request is an event, routed to an ordinary event reducer that does admission — NOT a new "spawn
handler" concept.** Operator (Q1): "the spawn handler is just like any other event reducer." So a spawn goes
through the *normal event-dispatch/registry path* (§4): the kernel routes it to the event reducer that governs
the spawn contract, which inspects the requested `SpawnLimits` and returns admit / reject-with-reason. On
reject the spawn does not happen and the requester gets a rejection response (`Err(...)`, the same shape as an
event reducer declining any effect). On admit the kernel spawns with the resolved (clamped) limits.

This is the same event-dispatch mechanism as everything else — admission is a *policy* the platform hosts as a
(Cadenza) event reducer, over the kernel's spawn mechanism — so it ties directly into the default-event-handler
routing ([[DESIGN-default-event-handler-guest]]) rather than introducing a parallel handler type. The spawn
contract is just one more contract the event registry maps to a handler.

### 3. A spawn WIT capability

Spawn becomes a typed host import, wired only on the privileged world — exactly as `deliver` is:

```
interface spawn {
  use types.{program-hash, reducer-id};
  use reducer.{...};
  record spawn-limits { max-linear-memory-bytes: u64, max-yields: u64 }
  record spawn-links { parent-watches-child: bool, child-watches-parent: bool }   // Q4: links, yes
  record spawn-request { program: program-hash, nonce: list<u8>, limits: spawn-limits, links: spawn-links }
  spawn: func(request: spawn-request) -> result<reducer-id, spawn-error>;   // reject = the admission reducer declined
}
// Q4 (operator): a request MAY set `links` but NOT `kind` — the child's kind is platform-assigned /
// privilege-controlled, never caller-chosen (choosing kind would be a privilege-granting act).
```

A privileged reducer that holds the `spawn` import can request a spawn (with limits); the call routes through
the normal event-dispatch path to the event reducer governing the spawn contract for admission (§2), and
returns the new reducer-id or a rejection. **v-inference owns the WIT synthesis** (`wit_world`), so the exact
record/variant shape is co-settled with them, as with the other host imports. `spawn` joins `deliver` in the
one privileged world (Q3).

### 4. The privilege split — privileged spawn, non-privileged effects

- **Spawn is privileged.** Only a privileged world imports `spawn`. A non-privileged (ordinary) reducer cannot
  name it — the same static, world-based gate as `deliver`.
- **Effects are the non-privileged floor.** An ordinary reducer's only outward action is emitting an effect (a
  Request); it cannot spawn or deliver. This sharpens the existing model: `deliver` (route) and now `spawn`
  (create + set limits) are the privileged acts; emitting an effect is what everyone can do.

**One coarse privileged world (Q3, operator-settled).** `spawn` and `deliver` travel together in the *single*
privileged (event) world — no fine-grained split (no spawn-but-not-deliver). Operator (Q3): "we don't need
fine granularity on worlds. once the event reducers get implemented they aren't going to change all that often
and they're going to be tightly controlled." So the world set stays two-way — ordinary (effects only) and
privileged (spawn + deliver + graph + provenance) — and the privileged world simply gains `spawn` alongside
its existing privileged imports. Coarse is acceptable precisely because privileged event reducers are stable
and tightly controlled.

## How a spawn flows (end to end)

1. A privileged reducer calls `spawn(request)` (the WIT import), naming a program + requested `SpawnLimits`.
2. The kernel routes the request through event-dispatch to the **event reducer governing the spawn contract**
   for admission (policy): admit / reject-with-reason (Q1).
3. On admit, the kernel resolves effective limits = requested **clamped to** the node ceiling, and launches
   the reducer, threading the resolved `SpawnLimits` on `SpawnContext` → `arm_store_safety` arms *that* store
   with *its* budget.
4. The `spawn` call returns the new `reducer-id` (or the rejection) to the caller.

## Coordination

- **v-inference** (`wit_world`): the `spawn` WIT interface synthesis — co-settle the `spawn-request` /
  `spawn-limits` / `spawn-error` shapes, as with `deliver`/`graph`.
- **v-platform-itest**: a conformance run once built — a privileged guest spawns with limits; a spawn handler
  admits/rejects; assert the routed spawn + the armed budget (observable as a runaway that traps at the
  per-spawn ceiling, not the node one).
- This composes with the **default-event-handler** rework ([[DESIGN-default-event-handler-guest]]): admission
  IS event-dispatch to an ordinary event reducer (Q1), so it rides the same routing, not a parallel handler.

## Build sequence (greenlit on Q1 + Q3)

Ordered; the first slice is gated on #3209 merging (it extends `arm_store_safety`'s config threading), the WIT
slice is co-owned with v-inference.

1. **`SpawnLimits` threading** *(after #3209 merges)* — add `SpawnLimits { max_linear_memory_bytes, max_yields }`
   to `Spawn`; carry the resolved (clamped-to-node-ceiling, Q2/Q5) limits on `SpawnContext`; have
   `arm_store_safety` use the per-spawn budget instead of the uniform node value. The host-side clamp (Q2) lives
   here — a hard backstop under the admission reducer. Pure kernel plumbing on top of #3209.
2. **The `spawn` WIT** *(with v-inference)* — add the `spawn` interface to the one privileged world (Q3),
   alongside `deliver`. Co-settle the record/variant shapes. The request carries `program` + `nonce` + `limits`
   + **`links`** (Q4); the child's **`kind` is NOT caller-settable** (Q4 — platform-assigned, privilege-
   controlled). **v-inference confirmed this needs zero
   new front-end synthesis** — it is structurally the deliver-response shape already handled (#3133/#3137/#3171):
   `spawn(request: record) -> result<reducer-id, spawn-error>` maps like `deliver-response`, and a new `spawn`
   interface in the privileged world synthesizes with no per-interface arm. `program-hash`/`reducer-id` are
   `hash` = `list<u8>` → `Bytes`. **The one requirement:** the anonymous `spawn-error` world variant needs a
   Cadenza decl identity, so a guest performing `spawn` must declare a named `type SpawnError = | … |` whose
   case-set matches the variant (kebab-normalized) — exactly like `deliver`'s `Error`; absent it, the op is
   skipped (graceful, hand-declare). When the custom prelude lands, `SpawnError` can live there ambiently
   (v-inference will extend `guest_sum_names` to scan it). Co-settle the `spawn-error` case-set with
   v-inference when the shape finalizes.
3. **Admission via event-dispatch** (Q1) — route a spawn request to the event reducer governing the spawn
   contract; admit → kernel spawns with resolved limits, reject → rejection response. Rides the event registry
   + default-event-handler routing.
4. **Conformance run** (v-platform-itest) — a privileged guest spawns with limits; the governing event reducer
   admits/rejects; assert the routed spawn + that the child is armed with its per-spawn budget (a child that
   exceeds its own ceiling traps, distinct from the node ceiling).

## Open questions

1. ~~Spawn handler = event handler, or its own handler?~~ **Resolved (Q1)** — it is an *ordinary event
   reducer* reached via the normal event-dispatch/registry path; no new handler concept.
2. ~~Clamp *and* handler, or handler only?~~ **Resolved (Q2)** — a host-side clamp is a hard backstop
   *underneath* the admission reducer (operator: "it's fine to have an additional clamp from the host"):
   defense in depth, so a buggy/absent admission reducer cannot grant more than the node permits.
3. ~~One privileged world, or finer capabilities?~~ **Resolved (Q3)** — one coarse privileged world carrying
   `spawn` + `deliver` together; no spawn-but-not-deliver split (privileged event reducers are stable +
   tightly controlled).
4. ~~What may a spawn request set beyond limits?~~ **Resolved (Q4)** — a request MAY set the child's `links`
   but NOT its `kind` (operator: "we definitely need links. I don't think it should be able to specify the
   kind though"). So the request is `program` + `nonce` + `limits` + `links`; `kind` is platform-assigned
   (privilege-controlled), never caller-chosen.
5. ~~Per-node / per-spawn relationship~~ **Resolved (Q5)** — per-node `ResourceLimits` is the default +
   ceiling; a per-spawn request is honored within that ceiling.
