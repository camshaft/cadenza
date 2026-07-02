# Capability — Memory And Resource Model

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This
> document defines how a program manages memory and how its resource use is accounted, without a
> garbage collector and with deterministic cleanup. Requirements realize
> [Core Principle III](../../constitution.md) and [Core Principle V](../../constitution.md) and
> trace to [overview §4](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying
> exactly one obligation, under a stable heading.

## Purpose And Scope

This capability fixes the memory and resource discipline of a Cadenza program. It requires that the
runnable form need no tracing garbage collector, that the point at which a value's storage is
released be a deterministic function of the source, and that allocation be accountable against the
deterministic resource measure. It states these as invariants; the concrete ownership discipline
that realizes them is an implementation choice constrained by them.

## No Garbage Collection

### The Runnable Form Needs No Collector

The runnable form of a program MUST NOT depend on a tracing garbage collector for correctness.

The timing of memory reclamation MUST NOT be a source of nondeterminism in a program's observable behavior.

## Deterministic Cleanup

### Cleanup Is Source-Determined

The point at which a value's storage is released MUST be a deterministic function of the source.

A value's storage MUST be released after its last use in a way the executable semantics defines, rather than at an unspecified later time.

## Bounded Allocation

### Allocation Is Accountable

Allocation performed by a running component MUST be accountable against the deterministic resource measure.

A program MUST NOT be able to allocate unboundedly without consuming the deterministic resource measure.

## Aliasing

### Aliasing Is Statically Disciplined

A value MUST NOT be observably mutated through one reference while it is read through another in a way the executable semantics leaves unspecified.

The compiler MUST reject a program whose aliasing the memory discipline cannot establish as safe, rather than emit a component with unspecified aliasing behavior.
