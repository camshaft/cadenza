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

### Exhaustion Is Observed As A Trap In A Derived Component

A derived component that exhausts the deterministic resource measure MUST halt as a trap, so that the reference interpreter's exhaustion terminal condition and the derived component's trap are judged as agreement rather than as a divergence when a compiled program is checked against the oracle.

A derived component MUST NOT be required to distinguish exhaustion from a trap in its observable behavior, because the component boundary signals a bounded halt as a trap and carries no separate exhaustion outcome.

### An Unsupported Construct Is Declined, Not Miscompiled

A generation whose compiler does not yet compile a construct a program uses MUST decline to derive a component for that program rather than emit a component whose observable behavior diverges from the reference interpreter.

The set of programs a generation's compiler declines to derive MUST be observably distinct from the set whose derived component diverges from the oracle, so that a compiler grown incrementally is measured by the programs it compiles in agreement rather than by masking a divergence as an absence.

### An Offered Interpreted Derivation Agrees With A Generated One

When a generation offers interpreted derivation in addition to generating components, a component derived by embedding the reference interpreter and a component derived by generation MUST exhibit the same observable behavior for the same program.

A component derived by embedding the reference interpreter MUST satisfy the determinism, capability-binding, bounded-termination, and reproducibility guarantees identically to a generated component.

### The Generated Path Is Exercised Before It Is Trusted

Component generation MUST be exercised against the oracle on a real derived-and-run component before a generation relies on it, so that the generated path is proven to materialize rather than deferred indefinitely.

The generated path MUST be exercised against the oracle over every executable-semantics case the generation's compiler compiles, so that oracle agreement is measured across the corpus as the compiler grows rather than on a single derived component.

## The Staged Path

### The Seed Interpreter Is The One Step Outside The Loop

The seed reference interpreter MAY be authored in a foreign language because no Cadenza toolchain yet exists to derive it.

The seed reference interpreter MUST derive a component that satisfies the same guarantees as any later generation's output.

### Each Generation Is Derived By The Previous

Each generation of the toolchain after the seed MUST be derivable by the generation before it.

The first Cadenza artifact the bootstrap targets MUST be a compiler authored in Cadenza, derived by running the seed reference interpreter over its source, rather than a Cadenza-authored interpreter.

The translation of a program's canonical representation to component bytes MUST be authored in Cadenza rather than in the seed language, so that the seed contributes only evaluation and the compiler it runs contributes the compilation.

Every stage that lowers a program toward component bytes, including any assembly of a textual or structured instruction form into its binary encoding, MUST be authored in Cadenza rather than performed by a seed-language or external tool, so that no part of the translation escapes the Cadenza-authored compiler.

The seed MUST realize a byte-sequence value form, so that the Cadenza-authored compiler it runs can construct component bytes as an ordinary value rather than through a seed-language translation.

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
