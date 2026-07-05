# Effects Model — Choice: algebraic-one-shot

> **The default choice for the `effects-model` decision** (see [README.md](./README.md) for the
> decision and the requirements a choice must satisfy). It pins the concrete operational mechanism for
> effects: how a host import suspends and resumes, and how an intra-program effect is handled. The
> guarantees it must satisfy — escaping row equals the manifest, purity is the empty row, determinism —
> are not replaceable.

## Host imports suspend by host-owned replay

Every imported host function is a **suspending boundary effect**. The mechanism is Temporal-style
replay, and its defining property is that **the program holds no resume state — the host owns the log**.

- The program's entry is re-invoked identically every time: `run(input)`. It carries no cursor, no
  continuation, no log.
- The host owns three pieces of state, and they live **entirely host-side** (see "Where the context
  lives" below): `log` (the ordered responses to host calls resolved so far), `index` (how far replay
  has advanced this invocation, reset to 0 at each invocation), and `pending` (the frontier call, if
  any).
- When the guest calls an imported host function `f` with `args`, the host's implementation of `f`:
  - if `index < len(log)`: returns `log[index]` and advances `index` — this is **replay**, and the
    real side effect is **not** performed again;
  - otherwise (`index == len(log)`, the frontier): records `pending = (f, args)` and unwinds with a
    sentinel, so the invocation stops.
- The driver that called `run(input)` observes either `Done(result)` or the sentinel-with-`pending`.
  On the latter it takes `pending = (f, args)`, hands `(f, arg-bytes)` up to the initial callsite as an
  **opaque binary** (see below), lets the host resolve it however it likes (async / federated / later /
  on another machine), appends the response to `log`, and re-invokes `run(input)` from the start.
  Determinism (constitution III) guarantees the replay makes the same call sequence, so it fast-forwards
  one call further and either finishes or hits the next frontier.

The continuation is therefore exactly **(content-addressed component + input + log)** — all canonical
data, no serialized linear memory — which is why a run resumes on any federated host. The corpus
`(host-responses …)` fixture *is* this log.

## The suspend token is an opaque binary that propagates to the initial callsite

The program entry's result type gains a suspension arm (a coordinated change to the frozen
`component-abi.md`): `run(input) -> Done(result-bytes) | Suspended(fn-id, arg-bytes) | trap`. The
`arg-bytes` are the WIT-typed arguments serialized by the [type-mapping](../type-mapping/) choice —
**opaque to the ABI**, meaningful only to the target that resolves the call. When a Cadenza program
invokes another Cadenza program as a tool, the inner `Suspended` propagates up unchanged, so a nested
suspension reaches the top-level federated dispatcher as one opaque token. The host does not need a
program counter for "where it got to" — the length of `log` is that position; the token is what the
host ships to the executor and correlates the returned response against.

## Where the host context lives (no threading through WIT)

The `log` / `index` / `pending` are **not** part of the WIT world and are **never** threaded through
imported calls. They are host runtime state, scoped to a single invocation:

- The runtime engine is global and shared; **per-invocation host state lives in the store the runtime
  gives each instantiation** (in the default engine, a Wasmtime `Store<T>`, whose data `T` the host
  defines). Host state is per-store, not global to the runtime.
- Each imported host function is a host-side closure that receives mutable access to that store data,
  so it can read `log`, advance `index`, and set `pending`. Host callbacks **can** mutate this state,
  and it is isolated per invocation, not shared across concurrent runs.
- Consequently the guest's import signature stays the clean, strongly-typed WIT `f : A -> B` with **no
  extra context parameter and no host handle**. WIT does not need a mechanism to thread an unknown host
  reference through calls, because the guest holds no host reference at all — the host-owns-the-log
  decision is exactly what removes that need. (WIT resources would be the mechanism *if* the guest had
  to hold a host handle; it does not.)

This keeps the suspension invisible to the guest: a host call is written in ordinary direct style
(`let x = ask(y)`), there is no `async`/`await` in the language, and durability is purely a
host-plus-determinism property.

## Replay is the semantics; the host chooses the resumption strategy per call

Host-owned replay defines the **observable behavior** — a run is a deterministic function of
`(component, input, log)`, and that triple is the portable, canonical-data continuation that survives
a crash and migrates to another federated host. But *how* the host resumes at a given suspension point
is the **host's runtime choice**, off the byte path, because determinism makes every faithful strategy
produce byte-identical observable behavior. The host may pick, per call, whichever is cheapest:

1. **Answer in-process (no suspend).** The host has a cheap, local, deterministic answer and returns it
   synchronously from the import; the guest never observes a suspension. Fastest; no checkpoint.
2. **Checkpoint and resume live (no teardown).** The host records the response to the durable log
   **and** resumes the live instance in place — keeping linear memory hot and avoiding re-execution.
   Durable and fast: good when the answer is available locally now but the host still wants a resumable
   point.
3. **Checkpoint and tear down.** The host yields the opaque `Suspended` token, drops the instance, and
   resolves the call async / federated / later; it resumes by replay from the log, possibly on a
   different machine. The portable fallback, required whenever the answer is not available now or the
   work must migrate.

The soundness constraint that ties the three together: **the response the host feeds in any mode MUST
be the value it would record in the log**, so a locally-resumed run and a torn-down-then-replayed run
are observationally identical. The live instance (strategies 1–2) is a performance cache; the log is
the durable continuation. A host that resumes live without recording the response simply forfeits
migration *after* that point — its choice, not the language's.

The two mechanisms both exist in the default engine: in-place resume without teardown is an async host
function backed by a fiber/stack switch (or the component-model async task/subtask ABI), and teardown
is returning the `Suspended` arm and re-instantiating later. The guest program and its emitted bytes
are identical regardless of which the host uses.

## Intra-program effects are algebraic handlers with one-shot continuations

An effect a program handles internally, and that therefore never escapes to the host, is an algebraic
operation discharged by a lexically scoped handler:

- Handler resolution is lexical and deterministic (constitution II, III) — the nearest enclosing
  handler for an operation, resolved at compile time.
- Continuations are **one-shot (affine)** by default: a handler resumes its continuation at most once.
  This keeps fuel accounting and reference counting sound (a multi-shot continuation would duplicate a
  suspended computation and its held resources). Multi-shot resumption is a recorded open point, not a
  default.
- A handled effect that never reaches a host import does **not** appear in the manifest; only the
  escaping row does. `State` (mutation) re-enters as a pure state-passing effect discharged by a
  handler, so mutation is expressible without making the heap mutable.

## The effect row is row-polymorphic and closed before the boundary

The effect row is tracked as a row (the same row machinery records open records — see
`type-system.md`). A function polymorphic over its effect row is monomorphized to a closed row before
it crosses the component boundary, so the emitted component's import world is a fixed set — the
manifest — with no row variable. Purity is the empty row: a component that imports nothing runs
straight to `Done` and never suspends, and the compiler itself is such a component.
