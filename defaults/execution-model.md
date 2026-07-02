# Execution Model — Declared Default

> **What this file is.** The concrete, technology-named realization of the execution-model
> *requirements* the specification states technology-neutrally. The specification says the runnable
> form of a program is a content-addressed binary component that imports only its declared host
> operations, runs behind a versioned host interface, executes deterministically, and is bounded by
> a deterministic resource measure (constitution IV, V, VI; host-interface-binding.md;
> determinism-and-fuel.md; build-tool-interface.md). It does not name an engine or a format, because
> a good specification states requirements and does not overprescribe a technology. This file names
> the default choice that satisfies those requirements.
>
> This is a **declared default**, not a requirement. Accept it, tune it, or delete `defaults/` to
> reinvestigate from first principles.

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
| Host interface | **`cadenza-host/1`** | The single interface version generation 0 provides; a component names it explicitly and a runtime refuses any other. |
| Core host operations | `read-projection`, `emit-event`, `read-blob`, `invoke-tool` | Representative operations a component may request; each bound only when the manifest grants it. The set is the host's, not the language's, and extends as the host interface grows. |

## Host-interface operations (the `cadenza-host/1` world)

- **`read-projection`** — read a projection the manifest grants, returning its current value as
  opaque bytes consistent with the events folded up to the point the component runs.
- **`emit-event`** — propose an event of a kind the manifest grants; the runtime stamps its
  ordering, verified caller, and content hash rather than accepting them from the component.
- **`read-blob`** — read a content-addressed blob, bound only when the manifest grants it.
- **`invoke-tool`** — invoke a tool the manifest grants; the path by which heavy or specialized
  work, including deriving another program's source with Cadenza itself, is carried to a
  participant equipped to run it.

The running system decides which of these operations a component may hold, from the component's
manifest. A system may, for example, bind a log-folding component none of the operations that would
introduce nondeterminism and let it read only its granted projections — but that restriction is the
system's policy over the manifest, not a rule the compiler enforces.

## What is frozen vs. chosen

- **Frozen (requirements, in the spec):** determinism, capability-binding, content-addressed binary
  component, bounded termination, versioned interface. These do not change with the engine.
- **Chosen (here, replaceable):** the engine and the concrete determinism configuration. The
  component format and the `cadenza-host/N` interface version are a coordinated change if altered,
  because derived components bind to them.
