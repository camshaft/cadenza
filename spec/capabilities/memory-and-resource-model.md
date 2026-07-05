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
deterministic resource measure. It further fixes the property that makes those guarantees
realizable without a collector: because a value's contents are fixed when it is created, the
runtime value heap a program forms is acyclic, and over an acyclic heap a reference-count discipline
reclaims every value with no cycle left uncollected — so the runnable form can carry its own
allocation and reclamation and the runtime need provide only raw memory. It states these as
invariants; the concrete ownership discipline that realizes them — for instance a reference-count
discipline with in-place reuse, or a region discipline — is a declared-default choice constrained by
them, not fixed here.

## No Garbage Collection

### The Runnable Form Needs No Collector

The runnable form of a program MUST NOT depend on a tracing garbage collector for correctness.

The timing of memory reclamation MUST NOT be a source of nondeterminism in a program's observable behavior.

### The Value Heap Is Acyclic

The heap of values a program forms at runtime MUST be acyclic, because a value's contents are fixed when it is created and no operation mutates an existing value to refer to a value created later.

A recursive definition MUST refer to itself through a static reference to code rather than through a value that points back into the heap, so that recursion introduces no cycle into the value heap.

The compiler MUST NOT emit a construct that forms a cycle among heap values, so that a reference-count reclamation discipline leaves no value uncollected.

### Reclamation Is Carried By The Runnable Form

The runnable form of a program MUST carry its own allocation and reclamation of values, so that the runtime it targets need provide only raw memory rather than a memory manager.

The reclamation the runnable form carries MUST account each allocation and release against the deterministic resource measure, so that carrying its own reclamation does not escape the bound on resource use.

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

The aliasing discipline MUST be one the compiler applies internally to reclaim and reuse storage, rather than a use-counting obligation the program's author writes, so that a program's author states no aliasing annotation to be memory-safe.

### Reuse Is Not Observable

When the compiler reuses a value's storage in place because no other reference to that value can observe the difference, that reuse MUST NOT change the program's observable behavior, so that reusing storage is a transparent optimization rather than a mutation of a value.

A decision to reuse a value's storage or to allocate fresh storage MUST be a deterministic function of the source, so that reuse does not introduce nondeterminism into a program's observable behavior.
