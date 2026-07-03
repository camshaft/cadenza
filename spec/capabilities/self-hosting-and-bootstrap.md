# Capability — Self-Hosting And Bootstrap

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This
> document defines the reference interpreter as the behavioral oracle, the two derivation modes, and
> the staged path by which the language comes to build itself. Requirements realize
> [Core Principle IX](../../constitution.md) and [Core Principle XIV](../../constitution.md) and
> trace to [overview §10](../overview.md), [overview §11](../overview.md), and
> [overview §15](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying
> exactly one obligation, under a stable heading. The concrete seed language and staging plan are
> pinned at the declared-default location.

## Purpose And Scope

This capability fixes how Cadenza's behavior is defined once and reused everywhere, and how the
language reaches the point of building itself. The single executable semantics is realized as a
reference interpreter that is the behavioral oracle; a compiled program must agree with it. A
component may be derived either by embedding that interpreter over source or by ahead-of-time
compilation, and the two must be behaviorally indistinguishable. From a foreign-language seed, each
generation of the toolchain is derived by the one before it until the compiler is authored in
Cadenza. It states these as invariants; the seed language and the staging plan are declared defaults.

## The Oracle

### The Reference Interpreter Realizes The Executable Semantics

The reference interpreter MUST implement exactly the behavior the executable-semantics corpus records.

The reference interpreter MUST be the single behavioral oracle against which a compiled program's behavior is judged.

### A Compiled Program Agrees With The Oracle

Oracle agreement is required by Core Principle XIV: a compiled program's observable behavior agrees with the reference interpreter over the same input.

A generation whose compiled output disagrees with the reference interpreter on any executable-semantics case MUST NOT be promoted.

## Derivation Modes

### A Derived Component Agrees With The Oracle

A component the toolchain derives MUST exhibit, for a given program, the same observable behavior as the reference interpreter over the same input.

Generating a component MUST be treated as producing the runnable form of the program under the one semantics, not as a second definition of the language.

### An Offered Interpreted Derivation Agrees With A Generated One

When a generation offers interpreted derivation in addition to generating components, a component derived by embedding the reference interpreter and a component derived by generation MUST exhibit the same observable behavior for the same program.

A component derived by embedding the reference interpreter MUST satisfy the determinism, capability-binding, bounded-termination, and reproducibility guarantees identically to a generated component.

### The Generated Path Is Exercised Before It Is Trusted

Component generation MUST be exercised against the oracle on a real derived-and-run component before a generation relies on it, so that the generated path is proven to materialize rather than deferred indefinitely.

## The Staged Path

### The Seed Interpreter Is The One Step Outside The Loop

The seed reference interpreter MAY be authored in a foreign language because no Cadenza toolchain yet exists to derive it.

The seed reference interpreter MUST derive a component that satisfies the same guarantees as any later generation's output.

### Each Generation Is Derived By The Previous

Each generation of the toolchain after the seed MUST be derivable by the generation before it.

The first Cadenza artifact the bootstrap targets MUST be a compiler authored in Cadenza, derived by running the seed reference interpreter over its source, rather than a Cadenza-authored interpreter.

A self-hosting generation MUST be a Cadenza compiler authored in Cadenza.

A self-hosting generation MUST be derivable by the previous Cadenza compiler.

### The Interpreter Is Proven Before It Is Relied On

The reference interpreter MUST reproduce every executable-semantics case the generation realizes, so that a green semantics suite proves the oracle before any generation is judged against it.

The reference interpreter MUST be exercisable directly over a program's canonical representation, so that proving it does not depend on first packaging it as a derived component.

## Turning The Flywheel Means Execution

### A Regeneration Is Derived, Gated, And Run

A claim that a new generation exists MUST be backed by a component that was actually derived, rather than by the emission of the events that would accompany a regeneration.

A new generation MUST have passed both gates before it is claimed to exist.

A new generation MUST have run before it is claimed to exist.

A generation whose behaving is demonstrated only by a stand-in that never executed the derived component MUST NOT be treated as a conforming generation.

### Every Generation Re-Demonstrates The End-To-End Path

Every promoted generation MUST demonstrate, on a real derived-and-run component, that its imports mirror its manifest.

Every promoted generation MUST demonstrate, on a real derived-and-run component, that re-deriving the same source reproduces a byte-identical component.
