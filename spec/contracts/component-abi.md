# Frozen Contract — Component ABI

> **FROZEN CONTRACT.** This document pins how Cadenza's type universe maps onto the host
> interface's type system, the calling convention across the component boundary, and the memory
> layout of values that cross it. It is the byte-level agreement that lets a component derived by
> one generation of the compiler interoperate with a component derived by another. It is versioned
> and changed only by the coordinated act described in the constitution's Governance Floors. Its
> requirements realize [Core Principle VI](../../constitution.md) and
> [Core Principle VII](../../constitution.md) and trace to [overview §5](../overview.md),
> [overview §6](../overview.md), and [overview §8](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence under a
> stable heading. This contract states the mapping's properties; the concrete type table that
> realizes it is pinned at the declared-default location.

## Purpose And Scope

Every value that enters or leaves a component crosses a boundary between Cadenza's types and the
host interface's types. For a component to be a stable, content-addressed artifact that outlives
the compiler that produced it, that crossing must be fixed: the same exported signature must lower
and lift the same bytes regardless of which compiler generation emitted it. This contract pins the
type mapping, the calling convention, the boundary layout, and the component's entry — the export
through which it is invoked. It does not pin the internal representation a component uses for values
that never cross a boundary, which the compiler is free to choose, nor the concrete entry signature
of each program shape, which is a declared-default choice.

## The Boundary Type Mapping

### Every Exported Type Has A Stable Boundary Representation

Each Cadenza type that may appear in an exported or imported signature MUST have a single stable representation in the host interface's type system.

A type that has no defined boundary representation MUST NOT appear in an exported or imported signature.

The boundary representation of a type MUST be a function of the type alone, independent of the compiler generation that emits it.

### Generics Do Not Cross The Boundary

The compiler MUST monomorphize every exported and imported signature to concrete types before emitting the component interface.

A generic definition MUST NOT appear in a component's interface.

## The Calling Convention

### Lowering And Lifting Are Fixed Inverses

The lowering of a Cadenza value to its boundary representation MUST be a total function fixed by this contract.

The lifting of a boundary representation to a Cadenza value MUST be the inverse of the pinned lowering for every value in the lowering's range.

The calling convention across the boundary MUST be a function of the declared signature alone, independent of compiler internals.

## The Component Entry

### A Component Exports A Defined Entry

A derived component MUST export an entry through which the runtime invokes it.

The compiler MUST determine a program's entry from the program rather than leave it implicit.

### The Entry Signature Crosses The Boundary By The Same Rules

The entry's parameter and result types MUST each have a boundary representation fixed by this contract.

The entry's input and output MUST lower and lift across the boundary by the same calling convention as any other boundary value.

### The Entry Defines The Input For Oracle Agreement

The input over which a compiled program's behavior is compared to the reference interpreter MUST be the input to the program's entry.

## Boundary Memory Layout

### Aggregate Layout Is Determined By Type

The byte layout of an aggregate value that crosses the boundary MUST be determined solely by its declared type.

The byte layout of an aggregate value that crosses the boundary MUST NOT depend on the order in which the compiler discovered or emitted its fields.

Padding and alignment inserted into a boundary aggregate MUST be a fixed function of the aggregate's declared type.

## Additive Evolution

### Additive Evolution Of This Contract

A change to this contract that alters the boundary representation, calling convention, or layout of an already-defined type MUST carry a version increment and a stated migration path, per the constitution's Governance Floor on the component ABI.

A change to this contract that only adds a boundary representation for a type that previously had none MUST be permitted as an additive change.
