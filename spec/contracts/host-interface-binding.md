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
> stable heading. This contract names the interface operations by their function, not by a concrete
> engine; the concrete host interface is pinned at the declared-default location.

## Purpose And Scope

A component interacts with its runtime only through host operations it imports, and it may reach
only what it imports. This contract fixes that a component's imports are exactly the capabilities
its manifest enumerates — no more, so there is no latent authority, and no fewer, so the manifest
does not overstate what the component can do. It binds the core host operations to their manifest
declarations, and it fixes that the compiler adds no capability the program did not declare — so that
the system running the component can decide what to allow from the manifest alone. It does not fix the
concrete host interface, which is a declared default, nor the manifest's own encoding, which the
capability specifications govern. Which capabilities a *particular kind of program* is permitted to
declare is a policy of the system that runs the component, not of this contract.

## Imports Mirror The Manifest

### Imports Mirror The Manifest Exactly

The set of host operations a component imports MUST equal the set of capabilities its manifest enumerates.

The compiler MUST NOT emit an import for a host operation the manifest does not enumerate.

The compiler MUST NOT emit a manifest entry for which no corresponding import is generated.

### Ungranted Access Is Rejected At Compile Time

A program that reaches a host operation its manifest does not enumerate MUST be rejected at compile time.

The compiler MUST NOT emit a component that would fail to instantiate because it imports an operation absent from its manifest.

## Core Host Operations

### Each Core Host Operation Has A Fixed Binding

An operation that reads a projection MUST be bound only when the manifest grants that projection.

An operation that emits an event MUST be bound only when the manifest grants that event's kind.

An operation that reads a content-addressed blob MUST be bound only when the manifest grants the blob-reading capability.

An operation that invokes a tool MUST be bound only when the manifest grants that tool.

## Capability Honesty

### The Manifest Makes Nondeterminism Legible

An operation whose result is a source of nondeterminism MUST be reachable only through a capability the manifest enumerates, so that a program's determinism is legible from its manifest.

The compiler MUST NOT grant a program a source of nondeterminism the program did not declare as a capability.

### Policy Over The Manifest Belongs To The Runtime

The compiler MUST surface a program's declared capabilities in its manifest without deciding which capabilities are permissible.

The compiler MUST NOT refuse a program solely because a capability it declares would be disallowed by a particular runtime's policy.

## Interface Versioning

### Manifest And Interface Version Travel Together

The emitted component MUST name the exact host-interface version its imports are bound against.

A component MUST NOT be bound against a host-interface version its manifest does not name.

## Additive Evolution

### Additive Evolution Of This Contract

A change to this contract MUST be additive with respect to already-derived components, or else carry an explicit version increment.

A change to this contract that is not additive with respect to already-derived components MUST carry a stated migration path.
