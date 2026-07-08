# Effects Model — Choice: algebraic-one-shot

> **The default choice for the `effects-model` decision** (see [README.md](./README.md) for the
> decision and the requirements a choice must satisfy). It pins the concrete operational mechanism for
> effects: how a host import is called at the boundary, and how an intra-program effect is handled. The
> guarantees it must satisfy — escaping row equals the manifest, purity is the empty row, determinism —
> are not replaceable.

## A host call is a plain imported-function call; the entry is a plain function

An imported host function is called in **ordinary direct style** — `let x = ask(y)` — and returns its
response to the program. The program's entry is a **plain function `input -> output`** (component-abi.md
§The Entry Is A Plain Function): it has **no** suspension arm, no injected trap arm, no resume parameter.
There is no `async`/`await` in the language and no continuation state the program carries or the ABI
transports.

The **only** cross-boundary guarantee the language makes is **determinism**: a run's observable behavior
is a deterministic function of its input and the **ordered responses** to the host calls it makes
(constitution III). The corpus `(host-responses …)` fixture *is* that ordered response sequence, and a
case asserts "given these responses in this order, the run produces this output" — a determinism
assertion that needs no suspension machinery to state or check.

## How the host resolves a call is host policy the language does not represent

Because the language mandates only determinism, *how* a host produces a call's response is entirely the
host's runtime choice, off the byte path — every faithful strategy yields byte-identical observable
behavior. A host may pick, per call, whichever is cheapest:

1. **Answer inline.** The host has a cheap local answer and returns it synchronously from the import.
   Fastest; nothing is suspended.
2. **Suspend a fiber and resume in place.** The host runs the instance under wasmtime's async support and
   the imported function is an `async` closure; when it awaits, wasmtime suspends the **fiber** — freezing
   the whole wasm stack in place — and resumes exactly where it left off when the future resolves. The
   run's live state (the frozen stack) IS the resume state; no re-execution, host-local, not portable.
   (`Config::async_support` + `wasmtime-fiber`.)
3. **Tear down and re-derive.** The host drops the instance and later re-invokes the entry with the same
   input, feeding the recorded responses in order; determinism guarantees the re-derivation makes the same
   call sequence and reaches the same point. Portable (survives a crash, migrates to another host) at the
   cost of re-execution; viable precisely because the run is a deterministic function of `(input,
   responses)`.

The choice is **not** part of the program's meaning, and the **same emitted bytes serve all three** — the
component entry is a plain function under every strategy. A host that suspends a fiber holds the run's live
state; a host that re-derives holds only `(input, ordered responses)`; the language requires determinism,
not statelessness, and leaves which to the host. The one soundness tie: whatever a host feeds as a call's
response on one strategy must be what it would feed on another, so a fiber-resumed run and a re-derived run
are observationally identical.

## Nothing is threaded through the WIT signature

Because a host call is a plain imported-function call, the guest's import signature is exactly the
operation's declared type — `f : A -> B` — with **no** extra context parameter, no host handle, no resume
token (host-interface-binding.md §A Host-Delegated Operation Imports Verbatim). Any state a host strategy
needs (a response log for re-derivation, a fiber for in-place suspension) lives **host-side**, scoped to
the invocation (in the default engine, the Wasmtime `Store<T>` data), never in the WIT world and never
threaded through a call. The guest holds no host reference at all.

## An effect and its operations are declared before they are performed

An effect is **declared** with an `(effect …)` form that names it and binds each of its operations to a
type. The declaration binds the effect name in its enclosing scope exactly as a `type` or a `module`
does, and the effect is a **record of operations** — so an operation is reached by member access,
`Effect.op`, the same accessor a prelude namespace uses (`List.at`, `Int64.max`). This keeps effects
uniform with the rest of the language: declaring an effect is declaring a named thing, and performing an
operation is a member access applied to arguments.

```
(effect Scope
  (op lookup (-> String (Option Ty)))   ; Scope.lookup : String -> Option Ty
  (op bind   (-> String Ty Unit)))      ; Scope.bind   : String -> Ty -> Unit
```

- **Perform.** `(Scope.lookup name)` performs the `lookup` operation of `Scope`. Its arguments are
  checked against the operation's declared parameter types, its result is the operation's declared
  result type, and performing it **adds `Scope` to the enclosing function's inferred effect row** (the
  row machinery of `type-system.md`, unchanged). Operations are qualified by their effect —
  `Scope.lookup`, `Unify.resolve` — so two effects that each declare a `resolve` never collide, and a
  handler arm names the operation it discharges unambiguously.
- **Handle.** The `(handle ((Effect.op (params…) body)…) body)` form discharges an effect: each arm
  binds one operation's parameters and, within its body, `resume` returns a value to the point that
  performed the operation. A handler need not name every operation the effect declares — an operation no
  enclosing handler discharges propagates outward — but an arm that names an operation the effect does
  **not** declare is rejected (`CDZ0403`), because the declaration is the closed set of an effect's
  operations. Discharging removes the effect's label from the row of the computation the handler wraps.

## An effect is routed to the host at the entrypoint, not marked at its declaration

An effect declaration is a **routing-agnostic contract** — it names the effect and types its operations,
and says nothing about where the effect is discharged (capabilities-and-effects.md §"Host-Binding Is A
Routing Decision Made At The Entrypoint"). The *same* declared effect may be handled in-program in one
program and delegated to the host in another; the declaration commits to neither:

```
(effect log (op emit (-> String Unit)))   ; a contract — where it goes is decided elsewhere
```

Routing is decided at the **entrypoint**, which delegates a set of effects to the host with a `host`
form — the boundary counterpart of `handle`. `handle` discharges an effect in-program; `host` discharges
it at the component boundary as a call to an imported host function the host resolves. Both are lexical:
an operation resolves to the nearest enclosing router, whether that router is a `handle` or the
entrypoint's `host` delegation.

```
(def (main)
  (host (log)              ; delegate `log` to the boundary within this body
    (log.emit "ready")))   ; resolves to the host; `log` enters the manifest
```

The consequences:

- An effect a nearer `handle` discharges is intra-program *for that performance*: it never reaches the
  delegation and never appears in the manifest. This is why interposition is free — a `handle` around a
  `host`-delegated effect simply wins, letting a test harness mock the boundary.
- An effect an entrypoint delegates and no nearer handler discharges is a boundary effect: reaching one
  of its operations is a suspension point, it appears in the escaping row, and the delegation enumerates
  it in the manifest. The `host` delegation is the grant.
- An operation reached at a point with **neither** an enclosing handler **nor** an enclosing delegation
  — so the effect would escape ungranted — is `CDZ0401`. This one check replaces both the former
  reached-but-undeclared-host `CDZ0401` and the former undischarged-intra `CDZ0402`: with host-binding
  relocated to the entrypoint, "no handler and no delegation" is a single condition.
- A `host` delegation naming an effect its body never reaches is `CDZ0404` (latent authority) — the
  manifest is exactly the delegated-and-reached effects, no more and no fewer.

There is still **one way** to declare an operation (inside an `(effect …)`) and **one way** to perform it
(`Effect.op`), whichever side of the boundary discharges it — but *which* side is now a property of the
**entrypoint's routing**, not of the effect's declaration. This is what decouples the effect's contract
from where it is serviced: a library performs and handles effects and never grants host access; only an
entrypoint's `host` form routes an effect to the boundary, so authority enters strictly from the top.

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
straight to its result and makes no host call, and the compiler itself is such a component.
