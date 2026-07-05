# Frozen Contract — Host Interface Binding

> **FROZEN CONTRACT.** This document pins the exact relationship between a program's declared
> capability manifest and the imports of the component the compiler emits — the boundary that
> makes "no ambient authority" a property of the artifact rather than a convention. It is versioned
> and changed only by the coordinated act described in the constitution's Governance Floors. Its
> requirements realize [Core Principle IV](../../constitution.md) and
> [Core Principle VI](../../constitution.md) and trace to [overview §6](../overview.md) and
> [overview §8](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence under a
> stable heading. This contract fixes only the *mechanism* — that a host import is a WIT-typed function
> the manifest enumerates — and names no concrete host function; the concrete host interface a target
> offers is pinned at the declared-default location.
>
> **Contract version: 2.** Version 1 enumerated four fixed core host operations (a projection read, an
> event emit, a blob read, a tool invocation). Version 2 removes that enumeration: those were a concrete
> choice of one target and are subsumed by the general rule that an import is any WIT-typed host function
> the manifest enumerates. **Migration:** each former named binding is an instance of the general rule,
> so every already-derived component still conforms — the change is additive with respect to derived
> bytes (see §Additive Evolution) and requires no re-derivation.

## Purpose And Scope

A component interacts with its runtime only through host operations it imports, and it may reach
only what it imports. This contract fixes that a component's imports are exactly the capabilities
its manifest enumerates — no more, so there is no latent authority, and no fewer, so the manifest
does not overstate what the component can do. It fixes that a host import is a WIT-typed function the
manifest enumerates, and that the compiler adds no capability the program did not declare — so that
the system running the component can decide what to allow from the manifest alone. It does not name a
concrete host function nor fix the concrete host interface, which are the target's concern recorded at
a declared default, nor the manifest's own encoding, which the capability specifications govern. Which
capabilities a *particular kind of program* is permitted to declare is a policy of the system that
runs the component, not of this contract.

## Imports Mirror The Manifest

### Imports Mirror The Manifest Exactly

The set of host operations a component imports MUST equal the set of capabilities its manifest enumerates.

The compiler MUST NOT emit an import for a host operation the manifest does not enumerate.

The compiler MUST NOT emit a manifest entry for which no corresponding import is generated.

### Ungranted Access Is Rejected At Compile Time

A program that reaches a host operation its manifest does not enumerate MUST be rejected at compile time.

The compiler MUST NOT emit a component that would fail to instantiate because it imports an operation absent from its manifest.

## Imports Are WIT-Typed Host Functions

### A Host Import Is A WIT-Typed Function The Manifest Enumerates

A component's imports MUST be host functions declared in the WIT-shaped world it targets, each bound only when the manifest enumerates it.

An imported host function MUST carry a complete WIT-typed signature — its parameter types, its result type, and its error type — sufficient for the compiler to emit that import into the component's world without consulting anything outside the program's source.

The compiler MUST reject a program that imports a host function whose declared signature it cannot emit as a well-formed WIT import, rather than emit a component whose import does not match the world it names.

### Which Host Functions Exist Is The Target's Concern

Which host functions a world offers MUST be fixed by the target a component runs against and recorded at the declared-default location, rather than enumerated by this contract.

This contract MUST NOT name a concrete host function, so that the vocabulary of host operations can grow or differ per target without amending a frozen contract.

### The Manifest Is A Projection Of The Escaping Effect Row

A program's escaping effect row MUST equal the set of host functions it imports, so that the manifest is a projection of that row rather than a separately-asserted list.

A component that reaches no host function MUST have an empty manifest, so that a program's purity is the empty row and is legible from an empty manifest.

## Capability Honesty

### The Manifest Makes Nondeterminism Legible

An operation whose result is a source of nondeterminism MUST be reachable only through a capability the manifest enumerates, so that a program's determinism is legible from its manifest.

The compiler MUST NOT grant a program a source of nondeterminism the program did not declare as a capability.

### Policy Over The Manifest Belongs To The Runtime

The compiler MUST surface a program's declared capabilities in its manifest without deciding which capabilities are permissible.

The compiler MUST NOT refuse a program solely because a capability it declares would be disallowed by a particular runtime's policy.

## The Host Formats Nothing

### The Host Does Not Format A Component's Values

The host MUST NOT format a component's result or arguments into a display form of its own, so that the host carries no per-type rendering rules for the language's values.

The rendering of a value to its canonical text form and the reading of text to a value's canonical binary form MUST be operations the compiler exposes, not operations the host performs, so that "what a Cadenza value looks like" lives in the compiler rather than the host.

A harness that needs a component's typed result in displayable form MUST obtain it by invoking a compiler-provided display conversion over that typed result, rather than by inspecting and formatting the result itself, so that the harness stays value-agnostic while the result crosses the boundary as its proper type.

A tool that only loads, verifies, and runs components MUST remain unchanged by the addition of a new Cadenza value form, because it never renders those values, so that the value-form vocabulary can grow without touching the host.

## Interface Versioning

### Manifest And Interface Version Travel Together

The emitted component MUST name the exact host-interface version its imports are bound against.

A component MUST NOT be bound against a host-interface version its manifest does not name.

## Additive Evolution

### Additive Evolution Of This Contract

A change to this contract MUST be additive with respect to already-derived components, or else carry an explicit version increment.

A change to this contract that is not additive with respect to already-derived components MUST carry a stated migration path.
