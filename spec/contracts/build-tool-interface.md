# Frozen Contract — Build Tool Interface

> **FROZEN CONTRACT.** This document pins the interface Cadenza-the-compiler presents when it is
> invoked to derive a component: what it consumes, what it produces, and the properties it
> guarantees, so that a host can invoke it as a replaceable build tool and swap it for another
> language's tool without changing what loads and runs the result. It is versioned and changed only
> by the coordinated act described in the constitution's Governance Floors. Its requirements realize
> [Core Principle I](../../constitution.md), [Core Principle VI](../../constitution.md), and
> [Core Principle XIV](../../constitution.md) and trace to [overview §7](../overview.md) and
> [overview §9](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence under a
> stable heading. This contract names the tool's interface, not a concrete host; the concrete
> invocation surface is pinned at the declared-default location.

## Purpose And Scope

Cadenza is a build tool: something invokes it with a program's source and receives a runnable
component. This contract fixes that interface — the canonical source tree in, the component and its
manifest and diagnostics out — and the properties that make Cadenza a *replaceable* tool: it is
itself a verified, reproducibly-derived component, and it is not part of any minimal root that only
loads, verifies, and runs components. It also fixes that the component Cadenza produces may be
realized by embedding the reference interpreter over the source, so that a working component exists
before ahead-of-time compilation is complete. It does not fix the concrete invocation surface,
which is a declared default.

## What The Tool Consumes And Produces

### The Tool Consumes A Canonical Source Tree

The build tool MUST accept its input as a canonically encoded source tree as fixed by the source-tree-encoding contract.

The build tool MUST reject an input that is not a well-formed canonical source tree with a machine-readable diagnostic rather than an opaque failure.

### The Tool Produces A Component, A Manifest, And Diagnostics

The build tool MUST produce, on success, a content-addressed component together with the capability manifest against which its imports are bound.

The build tool MUST produce, on failure, machine-readable diagnostics rather than an opaque error.

The component the build tool produces MUST have imports that mirror the manifest it produces, as fixed by the host-interface-binding contract.

## The Tool Is Replaceable

### The Tool Is Itself A Verified Component

The build tool MUST be a content-addressed component.

The build tool MUST itself be reproducibly derivable from its own source.

The build tool MUST NOT be part of any minimal root whose only responsibilities are to load, verify, and run components.

### A New Source Language Is A New Tool

A minimal root whose only responsibilities are to load, verify, and run components MUST NOT contain a compiler for any source language.

A minimal load-verify-run root MUST be unchanged by the introduction of a build tool for a different source language.

## Derivation By Embedding The Reference Interpreter

### The Reference Interpreter May Be Bundled Or Linked

The build tool MAY derive a component by embedding the reference interpreter over the program's canonical source rather than by ahead-of-time compilation.

A component derived by embedding the reference interpreter MUST satisfy the determinism guarantee identically to a component produced by ahead-of-time compilation.

A component derived by embedding the reference interpreter MUST satisfy the capability-binding guarantee identically to a component produced by ahead-of-time compilation.

A component derived by embedding the reference interpreter MUST satisfy the reproducibility guarantee identically to a component produced by ahead-of-time compilation.

A component derived by embedding the reference interpreter MUST exhibit the observable behavior the reference interpreter defines for the program, so that the two derivation modes are behaviorally indistinguishable.

### The Embedded Interpreter Executes In The Component

A component derived by embedding the reference interpreter MUST compute the program's observable behavior by executing the interpreter over the program's canonical form when the component runs, rather than by replaying an observable behavior computed before the component ran.

A build tool MUST NOT emit, in place of the embedded interpreter, a component that reproduces only a pre-recorded transcript of the program's observable behavior.

The interpreter a derivation embeds MUST be the same across the programs it derives, so that a derived component's behavior is a function of the embedded program rather than of behavior-specific code the derivation emitted.

### The Interpreter And The Host Are Distinct Artifacts

The reference interpreter a component embeds MUST be an artifact distinct from the host that provides the capability operations the component imports.

The host that provides a component's imported capability operations MUST provide only the operations the component's manifest enumerates.

## Additive Evolution

### Additive Evolution Of This Contract

A change to this contract MUST be additive with respect to already-derived components and already-invoked tool interfaces, or else carry an explicit version increment.

A change to this contract that is not additive with respect to already-derived components and already-invoked tool interfaces MUST carry a stated migration path.
