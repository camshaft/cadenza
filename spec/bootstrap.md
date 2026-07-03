# Bootstrap — Seed Toolchain And The Line Of Sight To Self-Hosting

> **BOOTSTRAP SPECIFICATION.** This document defines how Cadenza comes to exist and comes to build
> itself: the seed toolchain that has no Cadenza to compile it, the reference interpreter that
> becomes the behavioral oracle, the two derivation modes, and the bar an ignition must clear — a
> real, executed derivation, not the appearance of one. It fixes the human-versus-toolchain seam
> that resolves the bootstrap regress and the ignition subset the seed gate checks. Requirements
> realize [Core Principle XIV](../constitution.md), [Core Principle IX](../constitution.md), and
> [Core Principle XV](../constitution.md) and trace to [overview §11](./overview.md) and
> [overview §15](./overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence under a
> stable heading. The concrete seed language and staging plan are pinned at the declared-default
> location.

## The Line Of Sight

The path from this specification to a Cadenza that builds itself is direct: a reference interpreter in
a foreign language realizes the executable semantics and is the behavioral oracle; that interpreter
derives a component from Cadenza source by embedding itself over the source, and the first Cadenza
source it derives is the Cadenza-authored compiler; agents extend the Cadenza source of the language
and its compiler, each generation derived by the one before it and passed through both gates; and
self-hosting is reached when the Cadenza compiler is itself derived by the previous Cadenza compiler,
with the foreign-language interpreter no longer on the critical path. The reference interpreter is the
one foreign-language artifact; the compiler is the first Cadenza artifact, authored directly rather
than by way of a Cadenza-authored interpreter. The concrete realization of each step is the declared
bootstrap-strategy default.

## The Human-Versus-Toolchain Seam

### The First Toolchain Is Operator-Synthesized

The first Cadenza toolchain MAY be authored in a foreign language, because no Cadenza toolchain yet exists to derive it.

The first toolchain's output MUST satisfy the same determinism, capability-binding, bounded-termination, and reproducibility guarantees required of every later generation.

### The Toolchain Builds The Next Generation, Not Itself At Genesis

The seed toolchain MUST be able to derive a later generation of the toolchain from Cadenza source.

The seed toolchain MUST NOT be required to compile itself, so that the regress of "Cadenza needs Cadenza to build" is resolved by the operator-synthesized seed rather than by circular derivation.

## The Reference Interpreter As Oracle

### The Interpreter Realizes The One Semantics

The reference interpreter MUST implement exactly the behavior the executable-semantics corpus records.

The reference interpreter MAY remain authored in the foreign seed language, because the first Cadenza artifact the bootstrap targets is the compiler rather than a Cadenza-authored interpreter.

### Compiled Output Agrees With The Interpreter

An ignition MUST demonstrate that the compiled output of a program exhibits the same observable behavior as the reference interpreter over the same input, so that oracle agreement (Core Principle XIV) is exercised rather than assumed.

## Derivation Modes At Bootstrap

### The Seed Reference Interpreter Is Native And Is The Oracle

The seed reference interpreter MAY be a native program of the foreign seed language rather than a component, because its role is to define behavior and to run the Cadenza source of the first compiler, not to be a derived artifact itself.

The seed reference interpreter MUST be the behavioral oracle against which a derived component's observable behavior is judged.

### Compiled Derivation Produces The Component And Agrees With The Oracle

The toolchain MUST be able to derive a working component from a program's canonical source by generating the component, so that a runnable component exists without requiring the reference interpreter to be embedded in it.

A component the toolchain derives MUST exhibit the same observable behavior as the reference interpreter over the same input, on every executable-semantics case the generation realizes, before that generation is promoted.

### Interpreted Derivation Is An Optional Mode

The toolchain MAY additionally derive a component by embedding the reference interpreter over a program's canonical source, as an alternative to generating the component.

A component derived by embedding the reference interpreter MUST satisfy every guarantee a generated component satisfies.

## The Ignition Bar

### Ignition Demonstrates A Real End-To-End Derivation

An ignition MUST demonstrate that a Cadenza source program is derived to a content-addressed component.

An ignition MUST demonstrate that the derived component is actually run to produce its output.

An ignition MUST demonstrate that the derived component's imports mirror its declared capability manifest, so that the capability-binding is exercised rather than merely configured.

An ignition MUST demonstrate that re-deriving the same source with the same toolchain produces a byte-identical component, so that reproducibility is exercised rather than asserted.

### A Modeled Derivation Is Not An Ignition

An ignition demonstrated only by emitting the events that would accompany a derivation, without a component that was actually derived and run, MUST NOT be treated as a conforming ignition.

A subsystem whose specification can be satisfied without executing it MUST be exercised by the ignition's end-to-end path rather than stood in for by a placeholder.

## Turning The Flywheel

### A Generation Is Synthesized, Derived, Gated, And Run

A new generation MUST be produced by reading this specification, synthesizing Cadenza source, deriving it with the previous generation, and passing both gates, rather than by hand-editing an emitted component.

The claim that the system has evolved MUST be backed by a component that was actually derived, rather than by the appearance of a regeneration.

The claim that the system has evolved MUST be backed by a derived component that is now running.

### The Whole Regeneration Is Auditable

Each step by which a generation is produced MUST be recorded so that the regeneration is reconstructable and independently verifiable.
