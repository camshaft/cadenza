# DESIGN — per-spawn resource limits, a spawn WIT capability, and the privileged-spawn / non-privileged-effects split

**Owner:** v-platform (capability + dispatch model, `design/cadenza-platform.md` §3/§4/§5).
**Status:** sketch — operator-directed, pending steer. Builds on the per-node `ResourceLimits` of #3209.

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

**Resolution against the node.** The per-node `ResourceLimits` (#3209) becomes the **default and the ceiling**.
The effective limits a store is armed with = the spawn's requested limits, **clamped to the node ceiling** (a
spawn can never request *more* than the node permits) — but admission (below) can reject a spawn before it even
reaches clamping, so the two compose: the handler is policy, the clamp is a hard backstop. The resolved
`SpawnLimits` ride on `SpawnContext` into `arm_store_safety`, so a store is armed with *this reducer's* budget
instead of the uniform node value.

### 2. Admission control — the spawn handler

A **spawn handler** inspects a spawn's requested `SpawnLimits` and decides accept/reject — the operator's "the
event handler for that would be able to decide if those limits are ok." This mirrors the effect→event-handler
route (§4): a spawn request is routed to the handler that governs spawns (a registry default, like the
default event handler), which returns admit / reject-with-reason. On reject, the spawn does not happen and the
requester gets a rejection response (the same shape as the default event handler declining an effect —
`Err(...)`). On admit, the kernel spawns with the resolved (clamped) limits.

This makes admission a *policy* the platform hosts as a (Cadenza) handler, not hard-coded kernel logic — the
same "policy is a guest, mechanism is the kernel" split as the default-event-handler design
([[DESIGN-default-event-handler-guest]]).

### 3. A spawn WIT capability

Spawn becomes a typed host import, wired only on the privileged world — exactly as `deliver` is:

```
interface spawn {
  use types.{program-hash, reducer-id};
  use reducer.{...};
  record spawn-limits { max-linear-memory-bytes: u64, max-yields: u64 }
  record spawn-request { program: program-hash, nonce: list<u8>, limits: spawn-limits, /* kind, links … */ }
  spawn: func(request: spawn-request) -> result<reducer-id, spawn-error>;   // reject = the handler declined
}
```

A privileged reducer that holds the `spawn` import can request a spawn (with limits); the call routes through
the spawn handler for admission and returns the new reducer-id or a rejection. **v-inference owns the WIT
synthesis** (`wit_world`), so the exact record/variant shape is co-settled with them, as with the other host
imports.

### 4. The privilege split — privileged spawn, non-privileged effects

- **Spawn is privileged.** Only a privileged world imports `spawn`. A non-privileged (ordinary) reducer cannot
  name it — the same static, world-based gate as `deliver`.
- **Effects are the non-privileged floor.** An ordinary reducer's only outward action is emitting an effect (a
  Request); it cannot spawn or deliver. This sharpens the existing model: `deliver` (route) and now `spawn`
  (create + set limits) are the privileged acts; emitting an effect is what everyone can do.

This likely refines `ReducerKind` / the world set: the privileged (event) world gains `spawn` alongside
`deliver`; the ordinary world stays effects-only. Whether spawn and deliver are the *same* privileged world or
a finer split (a reducer that may spawn but not deliver, or vice versa) is an open question below.

## How a spawn flows (end to end)

1. A privileged reducer calls `spawn(request)` (the WIT import), naming a program + requested `SpawnLimits`.
2. The kernel routes the request to the **spawn handler** for admission (policy): admit / reject-with-reason.
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
- This composes with the **default-event-handler** rework ([[DESIGN-default-event-handler-guest]]): the spawn
  handler is the sibling of the event handler; both are policy-as-a-guest over a kernel mechanism.

## Open questions (for the operator)

1. **Spawn handler = event handler, or its own handler?** Is admission a distinct "spawn handler" in the
   registry, or a facet of the same default event handler? (Recommend: its own handler kind — spawn admission
   is a different decision than effect routing.)
2. **Clamp *and* handler, or handler only?** Is the node-ceiling clamp a hard kernel backstop *under* the
   handler's policy (recommended — defense in depth: a buggy/absent handler can't grant more than the node
   permits), or is the handler the sole authority?
3. **One privileged world, or finer capabilities?** Do `spawn` and `deliver` travel together in one privileged
   world, or should a reducer be able to hold one without the other (spawn-but-not-deliver)? This decides
   whether the world set stays two-way (ordinary / privileged) or grows.
4. **What may a spawn request set beyond limits?** Just `program` + `nonce` + `limits`, or also `kind` /
   `links` (does a privileged reducer choose the child's privilege)? Choosing the child's `kind` is itself a
   privilege-granting act and may need its own admission rule.
5. **Interaction with #3209's per-node config:** confirmed direction is per-node = default+ceiling, per-spawn =
   request-within-ceiling. (Recorded here as the working assumption.)
