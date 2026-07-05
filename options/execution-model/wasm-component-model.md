# Execution Model — Choice: wasm-component-model

> **The default choice for the `execution-model` decision** (see [README.md](./README.md) for the
> decision and the requirements a choice must satisfy). It is the concrete, technology-named
> realization of the execution-model requirements the specification states technology-neutrally: the
> runnable form of a program is a content-addressed binary component that imports only its declared
> host operations, runs behind a versioned host interface, executes with no undeclared
> nondeterminism, and is bounded by a deterministic resource measure (constitution IV, V, VI;
> host-interface-binding.md; determinism-and-fuel.md; build-tool-interface.md). It is a declared
> choice, not a requirement.

## The requirements this choice must satisfy (from the spec — do not weaken)

- **Content-addressed binary component behind a versioned host interface** (constitution VI;
  build-tool-interface.md §"The Tool Produces A Component, A Manifest, And Diagnostics").
- **Imports mirror the manifest exactly; no ambient authority** (constitution IV;
  host-interface-binding.md §"Imports Mirror The Manifest").
- **The compiler introduces no undeclared nondeterminism; a source of nondeterminism is reachable
  only through a declared capability** (constitution III; host-interface-binding.md §"Capability
  Honesty"; determinism-and-fuel.md).
- **Bounded termination by a deterministic resource measure** (constitution V;
  determinism-and-fuel.md §"Resource Accounting").

## The default choice

| Concern | Default | Why it satisfies the requirements |
|---|---|---|
| Component format | **WebAssembly Component Model** | Sandboxed, binary, content-addressable; imports are explicit and typed (maps directly to capability-scoping); no ambient authority by construction. |
| Runtime engine | an **embeddable component-model runtime** (default: **Wasmtime**) | Embeddable; supports the component model; supports a deterministic configuration (no wall-clock or entropy imports bound) and a deterministic resource bound (fuel metering). |
| Determinism config | execution **fuel-metered**; deterministic floating-point mode on; which nondeterministic capabilities a component may hold is left to the running system's per-role policy | The compiler introduces no undeclared nondeterminism and surfaces every requested capability; the running system decides from the manifest what to bind (for example, binding no clock or entropy for a log-folding role). |
| Resource measure | **fuel** (a deterministic per-instruction/per-call unit) | The deterministic resource measure the determinism-and-fuel contract requires; exhaustion halts at a defined point. |
| Host interface | a **versioned WIT-shaped world** the target defines; a component names its exact version explicitly and a runtime refuses any other | The mechanism, not a fixed vocabulary: a component imports the WIT-typed host functions its manifest enumerates from the world it names (host-interface-binding.md §Imports Are WIT-Typed Host Functions). |
| Host functions | **none fixed by the language**; a target's world defines its own | Which host functions exist is the target's concern (host-interface-binding.md §Which Host Functions Exist Is The Target's Concern). The illustrative `hivemind-host` world below is one target's set, not the language's — a program that imports none of them is pure (an empty manifest). |

## A target's host world is WIT-shaped (illustrative: `hivemind-host`)

The host interface a component targets is a component-model (WIT-shaped) world the *target* defines,
not the language. Cadenza fixes only the mechanism: a program declares each host function it imports
with a complete WIT-typed signature, and the compiler emits exactly those imports into the world the
component names (host-interface-binding.md §Imports Are WIT-Typed Host Functions). The bytes of each
type follow the [type-mapping](../type-mapping/) choice. A function is bound only when the component's
manifest enumerates it.

The world below is **illustrative** — the set one target, Hivemind, offers — shown so the mechanism is
concrete, not because the language fixes these functions. Another target defines a different world, and
a program that imports none of a world's functions is pure (an empty manifest).

```wit
// hivemind-host (one target's world — illustrative, NOT the language's vocabulary)
type projection-id = string          // names a projection the manifest grants
type kind = string                   // names an event kind the manifest grants
type blob-hash = list<u8>            // a content address
type tool-id = string                // names a tool the manifest grants

read-projection:  func(p: projection-id)          -> result<list<u8>, host-error>
emit-event:       func(k: kind, payload: list<u8>) -> result<_, host-error>
read-blob:        func(h: blob-hash)               -> result<list<u8>, host-error>
invoke-tool:      func(t: tool-id, request: list<u8>) -> result<list<u8>, host-error>
```

The running system decides which of a world's functions a component may hold, from the component's
manifest. A system may, for example, bind a log-folding component none of the functions that would
introduce nondeterminism and let it read only its granted projections — but that restriction is the
system's policy over the manifest, not a rule the compiler enforces.

Note that the Cadenza **compiler** is itself a component with an *empty* import world: it derives
programs as a pure `bytes → bytes` function and reaches no host function (build-tool-interface.md
§The Compiler Exposes Reader, Printer, And Display As Exports). Where a target offers a host function
that invokes another tool — including deriving a program's source with Cadenza itself — that is a
function of *that target's* world available to programs the target runs, not an import the compiler
holds.

### Derivation produces a real component whose world matches the manifest

Derivation produces a **real WebAssembly component** (not a bespoke core module): the Cadenza-authored
compiler's codegen emits the **complete component binary** — a core module plus the component-model
envelope (the component's type, canonical-ABI, and instance sections) — as an ordinary `Bytes` value,
so a program that declares `emit-event` yields a component whose world imports `emit-event` and a
program that declares no capability yields a world with no import. So "imports mirror the manifest
exactly" (host-interface-binding.md) holds **natively** — the world *is* the import set — with no
per-program import surgery (spec/learnings/2026-07-03-real-components-not-a-bespoke-module-model.md).

The whole component binary is produced by the Cadenza compiler itself (bootstrap.md §"The Compiler Is
Authored In Cadenza, Not In The Seed"), not by a separate wrapping tool in the byte path: a derivation's
byte output is a function of the Cadenza-authored compiler alone, which is what makes re-derivation
reproducible from that compiler and makes self-hosting a clean fixpoint (the compiler that emits complete
components can compile its own source into a compiler that does the same). The seed language provides
only the `Bytes` value form and a runtime to execute a finished component (below); it contains no
component encoder. `wasm-tools`/`wasm-encoder` may be used at the seed only as an out-of-band **oracle**
to validate that the Cadenza-emitted bytes are a well-formed component, never as a step that produces or
completes those bytes.

The **seed reference compiler (`cdz-rustc`) is a native program, not a component** — its role is to
lower Cadenza source to a real component and to compile `compiler.cdz`, so the seed's derivation is
compiled codegen; the behavioral oracle is the conformance corpus
(spec/learnings/2026-07-04-two-compilers-not-an-interpreter-and-a-compiler.md). Where a generation
*optionally* offers **interpreted derivation** — never as the runtime of a promoted generation
(bootstrap.md §"A Reference Interpreter Is An Optional Independent Oracle") — the same packaging
applies with the program's canonical AST embedded as component data the interpreter reads at run
time, and the interpreter code is identical across derived programs, so behavior comes from the
embedded AST rather than derivation-emitted per-program logic
(spec/learnings/2026-07-02-decouple-interpreter-wasm-from-host.md).

## Component entry shapes (per program shape)

A component exports a defined entry (component-abi.md §"The Component Entry"); its concrete signature
is pinned here per program shape. The entry's parameter and result cross the boundary by the type
mapping, and the entry's input is "the input" over which oracle agreement is judged.

```wit
// a fold-shaped program: prior projection state + a batch of events -> new state
fold:  func(state: list<u8>, events: list<list<u8>>) -> result<list<u8>, trap>

// a step-shaped program: a request -> emitted outcome (via emit-event) and a result
step:  func(request: list<u8>) -> result<list<u8>, trap>

// a tool-shaped program (e.g. Cadenza itself): a request -> a response
run:   func(request: list<u8>) -> result<list<u8>, trap>
```

A program shape and its entry name are a declared-default choice; a new shape is added here without
touching the frozen contract, which requires only that *some* defined entry exists and crosses the
boundary by the pinned rules.

## What is frozen vs. chosen

- **Frozen (requirements, in the spec):** determinism, capability-binding, content-addressed binary
  component, bounded termination, versioned interface. These do not change with the engine.
- **Chosen (here, replaceable):** the engine and the concrete determinism configuration. The
  component format and the `cadenza-host/N` interface version are a coordinated change if altered,
  because derived components bind to them.
