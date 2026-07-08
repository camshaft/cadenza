# Capability — Self-Hosting And Bootstrap

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This
> document defines the conformance corpus as the behavioral oracle, the two independent compiler
> implementations whose agreement supplies the judgment's independence, and the staged path by which
> the language comes to build itself. Requirements realize
> [Core Principle IX](../../constitution.md) and [Core Principle XIV](../../constitution.md) and
> trace to [overview §10](../overview.md), [overview §11](../overview.md), and
> [overview §15](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying
> exactly one obligation, under a stable heading. The concrete seed language and staging plan are
> pinned at the declared-default location.

## Purpose And Scope

This capability fixes how Cadenza's behavior is defined once and reused everywhere, and how the
language reaches the point of building itself. The single executable semantics is recorded by the
conformance corpus, which is the behavioral oracle; a compiled program must agree with it. The
judgment's independence comes from two implementations of the compiler — a foreign-language seed
compiler and the Cadenza-authored compiler — that must agree on every realized program, so that no
single implementation is both the definition of behavior and its own judge. Every program runs as a
component; there is one runtime. From a foreign-language seed, each generation of the
toolchain is derived by the one before it until the compiler is authored in Cadenza and compiles
itself. It states these as invariants; the seed language and the staging plan are declared defaults.

## The Oracle

### The Conformance Corpus Records The Executable Semantics

The conformance corpus MUST record, for each construct it covers, the observable behavior the executable semantics defines, so that the behavioral oracle is the reviewed record rather than any one program that computes it.

The recorded semantics MUST be the single behavioral oracle against which a compiled program's behavior is judged.

### A Compiled Program Agrees With The Recorded Semantics

Oracle agreement is required by Core Principle XIV: a compiled program's observable behavior agrees with the executable semantics, as recorded by the corpus, over the same input.

A generation whose compiled output disagrees with the recorded semantics on any executable-semantics case it realizes MUST NOT be promoted.

### Two Compilers Cross-Check Each Other

The independence of the judgment MUST be supplied by two implementations of the compiler — a foreign-language seed compiler and the Cadenza-authored compiler — whose compiled output MUST agree on the observable behavior of every program a generation realizes.

A generation whose two compiler implementations disagree on the observable behavior of any realized program MUST NOT be promoted, so that a divergence between independent implementations is caught rather than absorbed.

## Derivation Modes

### A Derived Component Agrees With The Oracle

A component the toolchain derives MUST exhibit, for a given program, the observable behavior the executable semantics records over the same input.

Generating a component MUST be treated as producing the runnable form of the program under the one semantics, not as a second definition of the language.

### An Unsupported Construct Is Declined, Not Miscompiled

A generation whose compiler does not yet compile a construct a program uses MUST decline to derive a component for that program rather than emit a component whose observable behavior diverges from the oracle.

The set of programs a generation's compiler declines to derive MUST be observably distinct from the set whose derived component diverges from the oracle, so that a compiler grown incrementally is measured by the programs it compiles in agreement rather than by masking a divergence as an absence.

### The Two Compilers Agree On Every Realized Program

The seed compiler and the Cadenza-authored compiler MUST, for every program a generation realizes, produce components that exhibit the same observable behavior for the same input.

A component produced by either compiler MUST satisfy the determinism, capability-binding, bounded-termination, and reproducibility guarantees identically, so that neither implementation is a lower-assurance path.

### The Generated Path Is Exercised Before It Is Trusted

Component generation MUST be exercised against the oracle on a real derived-and-run component before a generation relies on it, so that the generated path is proven to materialize rather than deferred indefinitely.

The generated path MUST be exercised against the oracle over every executable-semantics case the generation's compiler compiles, so that oracle agreement is measured across the corpus as the compiler grows rather than on a single derived component.

## The Staged Path

### The Seed Compiler Is The One Step Outside The Loop

The seed reference compiler MAY be authored in a foreign language because no Cadenza toolchain yet exists to derive it.

The seed reference compiler MUST produce a component that satisfies the same guarantees as any later generation's output.

### Each Generation Is Derived By The Previous

Each generation of the toolchain after the seed MUST be derivable by the generation before it.

The first Cadenza artifact the bootstrap targets MUST be a compiler authored in Cadenza, compiled to a component by the seed reference compiler, rather than a Cadenza-authored interpreter.

The translation of a program's canonical representation to component bytes MUST be expressible in Cadenza, so that a self-hosting generation performs the compilation in Cadenza rather than depending on the foreign-language seed for it.

Every stage that lowers a program toward component bytes, including any assembly of a textual or structured instruction form into its binary encoding, MUST be expressible in Cadenza, so that no part of a self-hosted translation escapes to a foreign-language or external tool.

The language MUST realize a byte-sequence value form, so that the Cadenza-authored compiler can construct component bytes as an ordinary value.

A self-hosting generation MUST be a Cadenza compiler authored in Cadenza.

A self-hosting generation MUST be derivable by the previous Cadenza compiler.

### Both Compilers Are Proven Against The Corpus Before They Are Relied On

The seed compiler MUST reproduce, on a real compiled-and-run component, the recorded behavior of every executable-semantics case the generation realizes, so that a green semantics suite proves it before any generation is judged against it.

The seed compiler MUST be exercisable directly over a program's canonical representation, so that proving it does not depend on any Cadenza-authored artifact existing first.

The Cadenza-authored compiler MUST, once it can compile a construct, reproduce that construct's recorded behavior and agree with the seed compiler on it, so that the two implementations are cross-checked as the Cadenza compiler grows.

## Turning The Flywheel Means Execution

### A Regeneration Is Derived, Gated, And Run

A claim that a new generation exists MUST be backed by a component that was actually derived, rather than by the emission of the events that would accompany a regeneration.

A new generation MUST have passed both gates before it is claimed to exist.

A new generation MUST have run before it is claimed to exist.

A generation whose behaving is demonstrated only by a stand-in that never executed the derived component MUST NOT be treated as a conforming generation.

### Every Generation Re-Demonstrates The End-To-End Path

Every promoted generation MUST demonstrate, on a real derived-and-run component, that its imports mirror its manifest.

Every promoted generation MUST demonstrate, on a real derived-and-run component, that re-deriving the same source reproduces a byte-identical component.
