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
>
> **Contract version: 2.** Version 2 adds the entry's suspension outcome (§The Entry May Suspend On A
> Host Call): the entry's result distinguishes normal completion, a trap, and a suspension carrying the
> pending host call, so that a host call is a suspension point resolved by replay
> (capabilities-and-effects.md §Every Host Call Is A Suspension Point). **Migration:** the suspension
> outcome is a new arm of the entry's result rather than a change to the normal-completion or trap
> representation, so a component that reaches no host function crosses the boundary exactly as under
> version 1; the increment is recorded because the entry's result type gains an arm. This precedes any
> deployed component, so no in-the-wild artifact requires re-derivation.
>
> **Contract version: 3.** Version 3 adds the value-heap runtime import (§The Value-Heap Runtime Crosses
> By A Well-Known Import): a derived program imports the single, well-known value-heap runtime interface
> to construct and render its runtime values, and a program's runtime values live in that runtime's
> linear memory and cross the internal runtime boundary as opaque handles, so the compiler emits programs
> against a shared runtime rather than open-coding a heap into every component. This runtime import is not
> a host function and not a capability (capabilities-and-effects.md §The Value-Heap Runtime Is The One
> Import That Is Not A Capability). **Migration:** the runtime import is a new, closed boundary — a
> program that produces only scalar/unit results and imports neither a host function nor the runtime
> crosses exactly as under version 2 — and the compound-result output convention changes from a
> component-owned `display()` resource to the runtime's `render` over a returned handle; this precedes any
> deployed compound-returning component, so no in-the-wild artifact requires re-derivation.

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

The input over which a compiled program's behavior is compared to the recorded corpus semantics MUST be the input to the program's entry.

### The Entry May Suspend On A Host Call

The entry's result MUST distinguish three outcomes — normal completion carrying the result value, a trap of a defined kind, and a suspension carrying the pending host call — so that a host call the run cannot resolve internally is returned to the host rather than blocked on inside the component.

A suspension outcome MUST carry the identity of the pending host function and its arguments in their boundary representation, and nothing that identifies where in the program's execution the call arose, so that the continuation is the host's response log rather than a position recorded in the component.

The host MUST resume a suspended run by re-invoking the entry with the same input, the run replaying to one host call further, so that the entry carries no resume parameter and re-invocation is the whole resume mechanism (capabilities-and-effects.md §Suspension Is Replay From The Host's Log).

## Boundary Memory Layout

### Aggregate Layout Is Determined By Type

The byte layout of an aggregate value that crosses the boundary MUST be determined solely by its declared type.

The byte layout of an aggregate value that crosses the boundary MUST NOT depend on the order in which the compiler discovered or emitted its fields.

Padding and alignment inserted into a boundary aggregate MUST be a fixed function of the aggregate's declared type.

## The Value-Heap Runtime

### The Value-Heap Runtime Crosses By A Well-Known Import

A derived program MUST reach its runtime values — constructing a compound value and rendering a value to its canonical text — through the single, well-known value-heap runtime interface it imports, rather than by open-coding a value heap into its own component, so that the heap representation is one shared artifact the compiler emits programs against.

The identity of that runtime interface MUST be fixed at the declared-default location and MUST be the same for every program a generation emits, so that any conforming host can satisfy the import and the interface is a stable part of the ABI rather than a per-program choice.

The concrete runtime a program is emitted against MUST be identified by the content address of that runtime component, so that a program's execution is deterministic in the pair (program, runtime content address) and a runtime built from different bytes is a distinct, explicitly-identified environment rather than a silent substitution (reproducible-derivation.md §Derivation Is A Function Of Source And Toolchain).

### The Emitted Component Records Its Required Runtime

A compiler MUST be built against a fixed runtime interface and a fixed runtime content address, so that which runtime a generation targets is a property of the compiler rather than a per-invocation choice, and the compiler and its runtime are one versioned pair.

A derived program MUST record, in the emitted component itself, the content address of the runtime it requires, so that the component is self-describing: what interface it imports and which exact runtime implementation satisfies that import both travel with the artifact.

### The Host Resolves The Runtime By Content Address

A host MUST resolve a program's runtime import by reading the required runtime content address the component records and locating the runtime component of that content address in a content-addressed store, rather than by assuming a single ambient runtime, so that programs pinned to different runtime versions coexist and each resolves the exact runtime it was emitted against.

A host that cannot locate a runtime of the content address a component requires MUST refuse to run the component rather than substitute a different runtime, so that a mismatched runtime is a detected error rather than a silent change in observable behavior.

### The Runtime Owns The Value Heap And Its Representation

The value-heap runtime MUST own the entire storage of a program's runtime values — their allocation, their in-memory layout, their reference-count discipline, and their reclamation — so that a program component holds no value storage of its own and the representation of every compound value is the runtime's private concern.

The internal representation a value has within the runtime MUST NOT be observable across the runtime boundary, so that the runtime may change how it lays out, shares, counts, or reclaims a value without altering any program's observable behavior or requiring a program to be re-derived.

### A Runtime Value Crosses As An Opaque Handle

A runtime value that crosses between a program and the value-heap runtime MUST cross as an opaque handle whose interpretation belongs solely to the runtime, so that the value's byte representation is the runtime's internal concern and never a layout the program or the host depends on.

The program MUST NOT dereference or interpret a runtime handle, so that the acyclic reference-counted heap the runtime owns is not aliased by another linear memory and the handle is a capability-free token rather than a pointer into shared state.

A runtime handle MUST be meaningful only within the single run and runtime instance that produced it, so that a handle is never part of a program's durable continuation (capabilities-and-effects.md §A Durable Continuation Is Canonical Data) and a resumed or replayed run reconstructs its values through the runtime rather than by carrying a handle across the boundary.

### A Compound Result Is Rendered By The Runtime

The observable result of a program that produces a compound value MUST be obtained by the host invoking the runtime's render over the program's returned handle, rather than by the program's own component owning a display of that value, so that the rendering of a value to its canonical text lives in the shared runtime the compiler emits (host-interface-binding.md §The Host Does Not Format A Component's Values).

The text the runtime's render produces MUST be the value's canonical text form under deterministic-value-form.md, so that a compound result crossing the boundary is byte-identical to the same value's recorded corpus form.

## Additive Evolution

### Additive Evolution Of This Contract

A change to this contract that alters the boundary representation, calling convention, or layout of an already-defined type MUST carry a version increment, per the constitution's Governance Floor on the component ABI.

A change to this contract that alters the boundary representation, calling convention, or layout of an already-defined type MUST carry a stated migration path, per the constitution's Governance Floor on the component ABI.

A change to this contract that only adds a boundary representation for a type that previously had none MUST be permitted as an additive change.
