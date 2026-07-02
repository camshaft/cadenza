# Capability — Verification Layers

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This
> document defines the optional, progressive verification layers above the mandatory core: contracts,
> refinement types, and machine-checked proof. Requirements realize
> [Core Principle VIII](../../constitution.md) and [Core Principle II](../../constitution.md) and
> trace to [overview §12](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying
> exactly one obligation, under a stable heading.

## Purpose And Scope

This capability fixes the layers a program may add above the mandatory floor of types, determinism,
and capability-safety, to state and discharge stronger properties: contracts (preconditions,
postconditions, invariants), refinement types, and machine-checked proof. It fixes that every layer
is optional and meaning-preserving, that a stated obligation is discharged or the program is
rejected, and — the load-bearing subtlety — that whether an obligation is discharged statically does
not change the bytes the compiler emits, so a nondeterministic solver never enters the reproducible
byte path.

## Layers Are Optional And Ordered

### A Program Compiles Without Any Layer

A program MUST compile when only the core guarantees — static typing, determinism, and capability-safety — are satisfied.

Engaging a verification layer MUST be something a program opts into, not a precondition of compiling.

### Layers Preserve Meaning

Adding a verification layer to a program that already compiles MUST NOT change the program's runtime meaning.

## Discharge

### A Stated Obligation Is Discharged Or Rejected

An obligation a program states MUST be either discharged or cause the program's rejection.

The compiler MUST NOT silently ignore an obligation it cannot discharge.

### Discharge Does Not Change Emitted Bytes

Whether an obligation is discharged statically MUST NOT change the bytes of the emitted component.

Whether a runtime check for an obligation is present MUST be determined by the program's explicit statement of it, not by whether a solver succeeded.

### Static Discharge Is A Reproducibly Checkable Certificate

A statically discharged obligation MUST be recorded as a certificate whose validation does not depend on a nondeterministic solver run.

A verifier MUST be able to check a discharge certificate reproducibly, obtaining the same result on every conforming run.

## The Contract Layer

### A Contract States A Checkable Condition

A precondition MUST state a condition required to hold when a function is entered.

A postcondition MUST state a condition required to hold of a function's result.

An invariant MUST state a condition required to hold of a type's every value.

### A Contract Is Discharged Dynamically Or Statically

A contract that is not statically discharged MUST be checked at runtime.

A runtime contract check that fails MUST raise a trap of a defined kind.

## The Refinement-Type Layer

### A Refinement Constrains A Base Type's Values

A value that satisfies a refinement's predicate MUST be admissible at that refinement type.

A value MUST be admissible at a refinement type only when it satisfies the refinement's predicate.

### Refinement Coercions Are Checked

Narrowing a base-type value to a refinement MUST be discharged, either statically or by a runtime check whose failure raises a trap of a defined kind.

Widening a refinement to its base type MUST require no check.

### A Refinement Erases To Its Base Type

A refinement MUST erase to its base type in the emitted component, so that it carries no runtime representation of its own.

## The Proof Layer

### A Proof Obligation Is Explicit

A machine-checked proof MUST discharge an explicitly stated obligation rather than an inferred one.

### A Proof Is A Reproducibly Checkable Witness

A discharged proof obligation MUST be recorded as a witness a verifier can check reproducibly, obtaining the same result on every conforming run.

A proof witness MUST be checkable without re-running the search that produced it.

## Optionality

### The Verification Layers Are Optional

Each verification layer above the mandatory core MUST be an optional capability a build may include or exclude, in accordance with the build's declared defaults.

### The Declared Default Is Include

When a build is not told whether to include a verification layer, it MUST include it.
